#!/usr/bin/env bash
set -euo pipefail

adapter_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)
source "$adapter_dir/build-env.sh"

fail() {
    printf '%s\n' "$*" >&2
    exit 1
}

[[ $(id -u) == 0 && -c /dev/ublk-control ]] || fail 'Run as root in the ublk guest.'
[[ $(findmnt -n -o FSTYPE -T /dev/shm) == tmpfs ]] || fail '/dev/shm must be tmpfs.'
[[ -d /sys/kernel/tracing/instances ]] || fail 'Guest tracefs instances are unavailable.'
command -v fio python3 >/dev/null

build_args=()
if (( $# != 0 )); then
    [[ $# == 1 && "$1" == learner ]] || fail 'Use queue-bench.sh or queue-bench.sh learner.'
    build_args=(--features learner-queue)
fi

run_dir=$(mktemp -d /dev/shm/io-journey-bench.XXXXXX)
build_dir=$(mktemp -d /var/tmp/io-journey-bench-build.XXXXXX)
trap 'rm -rf -- "$build_dir"' EXIT
printf 'Guest capture directory %s\n' "$run_dir"
{
    uname -srmo
    rustc --version
    fio --version
    printf '%s\n' 'One 32 MiB ublk device per run. Write back with fua=0. Release build.'
    printf '%s\n' 'Direct io_uring randrw, 70% reads, seed 20261120, 16 MiB span.'
    printf '%s\n' '2 MiB warmup, then 16 MiB measured, with end_fsync=0.'
    if (( ${#build_args[@]} != 0 )); then
        printf '%s\n' 'Uses the learner FTL, inner queue, and Linux dispatch policy.'
    fi
} > "$run_dir/environment.txt"

build_adapter "$adapter_dir" "$build_dir" --release "${build_args[@]}"
export IO_JOURNEY_ADAPTER_BIN="$build_dir/target/release/io-journey-ublk"

run_case() {
    local label=$1 policy=$2 depth=$3 size=$4
    mkdir "$run_dir/$label"
    printf 'Case %s\n' "$label"
    bash "$adapter_dir/smoke.sh" queue-case "$policy" "$depth" "$size" "$run_dir/$label"
}

run_case 01-fifo-4k-d1 fifo 1 4096
run_case 02-fifo-4k-d8 fifo 8 4096
run_case 03-read-first-4k-d8 read-first 8 4096
run_case 04-read-first-4k-d8 read-first 8 4096
run_case 05-fifo-4k-d8 fifo 8 4096
run_case 06-fifo-16k-d8 fifo 8 16384
python3 "$adapter_dir/report-queue.py" "$run_dir" | tee "$run_dir/report.md"
