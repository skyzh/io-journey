#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "$0")/.." && pwd -P)
tmp=$(mktemp -d "${TMPDIR:-/tmp}/io-journey-runner-test.XXXXXX")
trap 'rm -rf -- "$tmp"' EXIT

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

output=
for argument in "$@"; do
  case "$argument" in
    --output=*) output=${argument#--output=} ;;
  esac
done

socket_path=${NBD_URI#*socket=}
run_root=${socket_path%/*}
payload='libnbd: connected to private test socket\n{"jobs":[{"jobname":"flush"}]}\n'
if [[ -n "$output" ]]; then
  printf '%b' "$payload" >"$output"
else
  printf '%b' "$payload"
fi
touch "$run_root/fio.done"
if [[ ${FAKE_FIO_FAIL:-0} == 1 ]]; then
  exit 23
fi
SH
chmod +x "$tmp/bin/fio"

output_dir="$tmp/evidence"
runner_stderr="$tmp/runner.stderr"
if ! PATH="$tmp/bin:$PATH" \
  "$repo_root/run-fio.sh" "$tmp/bin/rust-nbd-lab" flush "$output_dir" \
  2>"$runner_stderr"; then
  fail "run-fio.sh rejected the deterministic fake fio contract"
  sed 's/^/  runner: /' "$runner_stderr" >&2
else
  if ! python3 - "$output_dir/fio.json" <<'PY'
import json
import pathlib
import sys

with pathlib.Path(sys.argv[1]).open() as source:
    document = json.load(source)
if document['jobs'][0]['jobname'] != 'flush':
    raise SystemExit('unexpected fio JSON payload')
PY
  then
    fail "fio.json is not strict parseable JSON"
  fi

  diagnostics_found=false
  if grep -q 'libnbd: connected to private test socket' "$runner_stderr"; then
    diagnostics_found=true
  fi
  for log in "$output_dir"/*.log; do
    if [[ -f "$log" ]] && grep -q 'libnbd: connected to private test socket' "$log"; then
      diagnostics_found=true
    fi
  done
  if [[ "$diagnostics_found" != true ]]; then
    fail "libnbd connection diagnostics were not preserved outside fio.json"
  fi
fi

failure_output="$tmp/failure-evidence"
failure_stderr="$tmp/failure.stderr"
if FAKE_FIO_FAIL=1 PATH="$tmp/bin:$PATH" \
  "$repo_root/run-fio.sh" "$tmp/bin/rust-nbd-lab" flush "$failure_output" \
  2>"$failure_stderr"; then
  fail "run-fio.sh hid the fake fio command failure"
else
  if [[ ! -f "$failure_output/fio.stdout.log" ]] || \
    ! grep -q 'libnbd: connected to private test socket' "$failure_output/fio.stdout.log"; then
    fail "failed fio stdout was not preserved"
  fi
  if ! grep -q 'fio failed; stdout preserved at' "$failure_stderr"; then
    fail "fio failure did not identify its preserved evidence"
  fi
fi

if [[ $failures -ne 0 ]]; then
  exit 1
fi

echo "run-fio contract checks passed"
