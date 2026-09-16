#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "$0")/.." && pwd -P)
tmp=$(mktemp -d "${TMPDIR:-/tmp}/io-journey-runner-test.XXXXXX")
runtime_parent=$(mktemp -d "${TMPDIR:-/tmp}/iojrt.XXXXXX")
trap 'rm -rf -- "$tmp" "$runtime_parent"' EXIT

failures=0
fail() {
  echo "FAIL: $*" >&2
  failures=$((failures + 1))
}

python3 - "$repo_root/fio/flush.fio" <<'PY' || failures=$((failures + 1))
import pathlib
import sys

options = {}
for raw_line in pathlib.Path(sys.argv[1]).read_text().splitlines():
    line = raw_line.strip()
    if not line or line.startswith(('#', ';', '[')) or '=' not in line:
        continue
    key, value = line.split('=', 1)
    options[key.strip()] = value.strip().lower()

enabled = {'1', 'true', 'yes', 'on'}
close_flush = (
    options.get('fsync_on_close') in enabled
    or options.get('end_fsync') in enabled
)
if not close_flush or options.get('fsync') in enabled:
    print('FAIL: flush job must request one close-time flush, not fsync after each write', file=sys.stderr)
    raise SystemExit(1)
PY

mkdir -p "$tmp/bin"

cat >"$tmp/bin/rust-nbd-lab" <<'PY'
#!/usr/bin/env python3
import json
import os
import pathlib
import socket
import sys
import time

if len(sys.argv) != 4 or sys.argv[1] != 'serve':
    raise SystemExit('expected: rust-nbd-lab serve ROOT CONNECTIONS')

root = pathlib.Path(sys.argv[2])
state_root = pathlib.Path(os.environ['FAKE_STATE_ROOT'])
state_root.mkdir(parents=True, exist_ok=True)
(state_root / 'server.pid').write_text(str(os.getpid()))
socket_path = root / 'nbd.sock'
listener = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
listener.bind(str(socket_path))
(root / 'listener.ready').touch()

deadline = time.monotonic() + 5
while not (root / 'fio.done').exists():
    if time.monotonic() >= deadline:
        raise SystemExit('timed out waiting for fake fio')
    time.sleep(0.01)

with (root / 'events.jsonl').open('w') as events:
    events.write(json.dumps({
        'seq': 1,
        'op': 'flush',
        'offset': 0,
        'length': 0,
        'status': 'ok',
    }, separators=(',', ':')) + '\n')

listener.close()
socket_path.unlink()
PY
chmod +x "$tmp/bin/rust-nbd-lab"

cat >"$tmp/bin/fio" <<'SH'
#!/usr/bin/env bash
set -euo pipefail

if [[ ${1:-} == --enghelp=nbd ]]; then
  exit 0
fi

socket_path=${NBD_URI#*socket=}
run_root=${socket_path%/*}
case ${FAKE_FIO_CASE:-prefix} in
  prefix)
    printf '%s\n' 'libnbd: connected to private test socket' \
      '{"jobs":[{"jobname":"flush"}]}'
    ;;
  trailing)
    printf '%s\n' '{"jobs":[{"jobname":"flush"}]}' \
      'libnbd: connection closed cleanly'
    ;;
  truncated)
    printf '%s' 'libnbd: connected'
    printf '\n%s' '{"jobs":[{"jobname":"flush"}]'
    ;;
  missing)
    printf '%s\n' 'libnbd: connected, but no document followed'
    ;;
  multiple)
    printf '%s\n' '{"jobs":[{"jobname":"flush"}]}' \
      '{"jobs":[{"jobname":"flush-again"}]}'
    ;;
  command-failure)
    printf '%s\n' 'libnbd: connection failed after partial output' \
      '{"jobs":['
    ;;
  *)
    echo "unknown fake fio case: ${FAKE_FIO_CASE:-}" >&2
    exit 2
    ;;
esac
touch "$run_root/fio.done"
if [[ ${FAKE_FIO_CASE:-} == command-failure ]]; then
  exit 23
fi
SH
chmod +x "$tmp/bin/fio"

expected_payload() {
  case $1 in
    prefix)
      printf '%s\n' 'libnbd: connected to private test socket' \
        '{"jobs":[{"jobname":"flush"}]}'
      ;;
    trailing)
      printf '%s\n' '{"jobs":[{"jobname":"flush"}]}' \
        'libnbd: connection closed cleanly'
      ;;
    truncated)
      printf '%s' 'libnbd: connected'
      printf '\n%s' '{"jobs":[{"jobname":"flush"}]'
      ;;
    missing)
      printf '%s\n' 'libnbd: connected, but no document followed'
      ;;
    multiple)
      printf '%s\n' '{"jobs":[{"jobname":"flush"}]}' \
        '{"jobs":[{"jobname":"flush-again"}]}'
      ;;
    command-failure)
      printf '%s\n' 'libnbd: connection failed after partial output' \
        '{"jobs":['
      ;;
  esac
}

assert_cleanup() {
  local case_name=$1
  local state_dir=$2
  local server_pid

  server_pid=$(<"$state_dir/server.pid")
  if kill -0 "$server_pid" 2>/dev/null; then
    fail "$case_name left its fake NBD server running"
  fi
  if [[ -n $(find "$runtime_parent" -mindepth 1 -print -quit) ]]; then
    fail "$case_name left a socket or temporary runtime behind"
  fi
}

assert_raw_log() {
  local case_name=$1
  local output_dir=$2
  if ! cmp -s "$output_dir/fio.stdout.log" <(expected_payload "$case_name"); then
    fail "$case_name did not preserve exact raw fio stdout"
  fi
}

run_success_case() {
  local case_name=$1
  local output_dir="$tmp/$case_name-evidence"
  local state_dir="$tmp/$case_name-state"
  local runner_stderr="$tmp/$case_name.stderr"
  mkdir -p "$state_dir"

  if ! FAKE_FIO_CASE="$case_name" FAKE_STATE_ROOT="$state_dir" \
    TMPDIR="$runtime_parent" PATH="$tmp/bin:$PATH" \
    "$repo_root/run-fio.sh" "$tmp/bin/rust-nbd-lab" flush "$output_dir" \
    2>"$runner_stderr"; then
    fail "$case_name: run-fio.sh rejected one complete fio JSON document"
    sed 's/^/  runner: /' "$runner_stderr" >&2
  elif ! python3 - "$output_dir/fio.json" <<'PY'
import json
import pathlib
import sys

with pathlib.Path(sys.argv[1]).open() as source:
    document = json.load(source)
if document['jobs'][0]['jobname'] != 'flush':
    raise SystemExit('unexpected fio JSON payload')
PY
  then
    fail "$case_name: fio.json is not the expected strict JSON document"
  fi

  assert_raw_log "$case_name" "$output_dir"
  assert_cleanup "$case_name" "$state_dir"
}

run_failure_case() {
  local case_name=$1
  local expected_status=${2:-1}
  local output_dir="$tmp/$case_name-evidence"
  local state_dir="$tmp/$case_name-state"
  local runner_stderr="$tmp/$case_name.stderr"
  local status
  mkdir -p "$state_dir"

  set +e
  FAKE_FIO_CASE="$case_name" FAKE_STATE_ROOT="$state_dir" \
    TMPDIR="$runtime_parent" PATH="$tmp/bin:$PATH" \
    "$repo_root/run-fio.sh" "$tmp/bin/rust-nbd-lab" flush "$output_dir" \
    2>"$runner_stderr"
  status=$?
  set -e

  if [[ $status -eq 0 ]]; then
    fail "$case_name: run-fio.sh accepted invalid or ambiguous output"
  elif [[ $expected_status -ne 0 && $status -ne $expected_status ]]; then
    fail "$case_name: expected status $expected_status, got $status"
  fi
  if [[ -e "$output_dir/fio.json" ]]; then
    fail "$case_name: failed run left a stale fio.json"
  fi
  assert_raw_log "$case_name" "$output_dir"
  if ! grep -Fq "stdout preserved at $output_dir/fio.stdout.log" "$runner_stderr"; then
    fail "$case_name: failure did not name its preserved raw log"
  fi
  assert_cleanup "$case_name" "$state_dir"
}

run_success_case prefix
run_success_case trailing
run_failure_case truncated 1
run_failure_case missing 1
run_failure_case multiple 1
run_failure_case command-failure 23

if [[ $failures -ne 0 ]]; then
  exit 1
fi

echo "run-fio contract checks passed"
