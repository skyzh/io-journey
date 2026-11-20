#!/usr/bin/env bash
set -euo pipefail

case_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)
ublk_dir="$case_dir/../ublk"
source "$ublk_dir/build-env.sh"
source "$ublk_dir/../mount-state.sh"

fail() {
    printf '%s\n' "$*" >&2
    exit 1
}

[[ ( $# == 0 || ( $# == 1 && "$1" == learner ) ) && $(id -u) == 0 &&
   -c /dev/fuse && -c /dev/ublk-control ]] ||
    fail 'Run in a private guest mount namespace with FUSE and ublk.'
[[ $(findmnt -n -o FSTYPE -T /dev/shm) == tmpfs && $(findmnt -n -o FSTYPE -T /run) == tmpfs ]] ||
    fail 'Keep /dev/shm and /run on tmpfs.'
require_private_mount_namespace
require_private_mount_parent /dev/shm

ublk_features=()
fuse_features=()
append_features=()
if (( $# == 1 )); then
    ublk_features=(--features learner-queue)
    fuse_features=(--features learner-journal)
    append_features=(--features learner-append)
fi

build_dir=$(mktemp -d /dev/shm/io-journey-append-build.XXXXXX)
trap 'rm -rf -- "$build_dir"' EXIT
capture=$(mktemp -d /dev/shm/io-journey-append-results.XXXXXX)
printf 'Guest capture directory %s\n' "$capture"
{
    uname -srmo
    rustc --version
    udevadm --version
    printf '%s\n' 'io-uring 0.7.16, fuser 0.16.0. Rust release builds.'
    printf '%s\n' 'Fresh 32 MiB ublk device with a write-back cache per case.'
    printf '%s\n' 'Four writes per wave, group sizes 4 and 32. 64 warmup and 256 measured 512-byte records.'
    if (( ${#append_features[@]} != 0 )); then
        printf '%s\n' 'Uses the learner FTL, queue, journal, recovery, and append engine.'
    fi
} > "$capture/environment.txt"

build_adapter "$ublk_dir" "$build_dir" --release "${ublk_features[@]}"
cargo build --locked --quiet --release --manifest-path "$case_dir/../fuse/Cargo.toml" "${fuse_features[@]}"
cargo build --locked --quiet --release --manifest-path "$case_dir/Cargo.toml" "${append_features[@]}"
export IO_JOURNEY_ADAPTER_BIN="$build_dir/target/release/io-journey-ublk"
export IO_JOURNEY_FUSE_BIN="$build_dir/target/release/io-journey-fuse"
export IO_JOURNEY_APPEND_BIN="$build_dir/target/release/io-journey-append"

for filesystem in ext4 fuse; do
    for group in 4 32; do
        run=0
        for engine in sync uring uring sync; do
            run=$((run + 1))
            label="$filesystem-$group-$run-$engine"
            output="$capture/$label"
            mkdir "$output"
            printf 'Case %s\n' "$label"
            bash "$ublk_dir/smoke.sh" append-case "$filesystem" "$engine" "$group" "$output"
        done
    done
done
python3 "$case_dir/report.py" "$capture" | tee "$capture/report.md"
