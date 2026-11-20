#!/usr/bin/env bash
set -euo pipefail

fuse_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)
backend=${1:-image}

if [[ $# -gt 1 || ( "$backend" != image && "$backend" != ublk ) ]]; then
    printf 'Use smoke.sh image or smoke.sh ublk.\n' >&2
    exit 1
fi
if [[ $(id -u) != 0 || ! -c /dev/fuse ||
      $(readlink /proc/self/ns/mnt) == "$(readlink /proc/1/ns/mnt)" ||
      $(findmnt -n -o FSTYPE -T /dev/shm) != tmpfs ]]; then
    printf 'Use guest root with /dev/fuse, /dev/shm on tmpfs, and a private mount namespace.\n' >&2
    exit 1
fi

build_dir=$(mktemp -d /dev/shm/io-journey-fuse-build.XXXXXX)
trap 'rm -rf -- "$build_dir"' EXIT
mkdir "$build_dir/tmp"
env CARGO_HOME="$build_dir/cargo" CARGO_TARGET_DIR="$build_dir/target" \
    TMPDIR="$build_dir/tmp" CARGO_INCREMENTAL=0 \
    cargo build --locked --quiet --manifest-path "$fuse_dir/Cargo.toml"

fuse_binary="$build_dir/target/debug/io-journey-fuse"
if [[ "$backend" == image ]]; then
    bash "$fuse_dir/workload.sh" "$fuse_binary"
else
    bash "$fuse_dir/../ublk/smoke.sh" fuse-case "$fuse_binary"
fi
