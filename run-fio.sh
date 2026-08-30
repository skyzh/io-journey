#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 3 ]]; then
  echo "usage: $0 LAB_BINARY {read|write|flush|trim} NEW_OUTPUT_DIR" >&2
  exit 2
fi

lab_binary=$1
experiment=$2
output_dir=$3
script_dir=$(cd "$(dirname "$0")" && pwd -P)

case "$experiment" in
  read|write|flush|trim) ;;
  *)
    echo "experiment must be read, write, flush, or trim" >&2
    exit 2
    ;;
esac

if [[ ! -x "$lab_binary" ]]; then
  echo "lab binary is not executable: $lab_binary" >&2
  exit 2
fi
if [[ -e "$output_dir" ]]; then
  echo "output directory already exists: $output_dir" >&2
  exit 2
fi
if ! command -v fio >/dev/null 2>&1; then
  echo "fio is required" >&2
  exit 2
fi
if ! fio --enghelp=nbd >/dev/null 2>&1; then
  echo "this fio build lacks the libnbd-backed nbd ioengine" >&2
  exit 2
fi

tmp_parent=${TMPDIR:-/tmp}
if [[ ! -d "$tmp_parent" || -L "$tmp_parent" ]]; then
  echo "temporary parent must be a real directory: $tmp_parent" >&2
  exit 2
fi
tmp_parent=$(cd "$tmp_parent" && pwd -P)

mkdir -m 700 "$output_dir"
run_root=$(mktemp -d "$tmp_parent/io-journey-nbd.XXXXXX")
chmod 700 "$run_root"
server_pid=
fio_pid=

cleanup() {
  if [[ -n "$fio_pid" ]] && kill -0 "$fio_pid" 2>/dev/null; then
    kill "$fio_pid" 2>/dev/null || true
    wait "$fio_pid" 2>/dev/null || true
  fi
  if [[ -n "$server_pid" ]] && kill -0 "$server_pid" 2>/dev/null; then
    kill "$server_pid" 2>/dev/null || true
    wait "$server_pid" 2>/dev/null || true
  fi
  case "$run_root" in
    "$tmp_parent"/io-journey-nbd.*) rm -rf -- "$run_root" ;;
    *) echo "refusing unexpected cleanup target: $run_root" >&2 ;;
  esac
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
trap 'exit 129' HUP

"$lab_binary" serve "$run_root" 2 &
server_pid=$!
for _ in $(seq 1 200); do
  [[ -S "$run_root/nbd.sock" ]] && break
  if ! kill -0 "$server_pid" 2>/dev/null; then
    echo "NBD server exited before its socket was ready" >&2
    exit 1
  fi
  sleep 0.01
done
if [[ ! -S "$run_root/nbd.sock" ]]; then
  echo "NBD socket did not become ready" >&2
  exit 1
fi

NBD_URI="nbd+unix:///?socket=$run_root/nbd.sock" \
  fio --eta=never --output-format=json \
  --output="$output_dir/fio.json" "$script_dir/fio/$experiment.fio" &
fio_pid=$!
wait "$fio_pid"
fio_pid=

wait "$server_pid"
server_pid=
if [[ -e "$run_root/nbd.sock" ]]; then
  echo "NBD server left its socket behind" >&2
  exit 1
fi
cp "$run_root/events.jsonl" "$output_dir/events.jsonl"
cp "$script_dir/fio/$experiment.fio" "$output_dir/job.fio"

if ! grep -q "\"op\":\"$experiment\"" "$output_dir/events.jsonl"; then
  echo "expected $experiment operation was not observed" >&2
  exit 1
fi

echo "evidence=$output_dir"
