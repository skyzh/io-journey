#!/usr/bin/env bash
set -euo pipefail

fuse_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)
source "$fuse_dir/../mount-state.sh"
backend=${1:-image}
course_mode=${2:-}
build_args=()
if [[ "$course_mode" == learner ]]; then build_args=(--features learner-journal); fi

if [[ $# -gt 2 || ( "$backend" != image && "$backend" != ublk ) ||
      ( $# == 2 && "$course_mode" != learner ) ]]; then
    printf 'Use smoke.sh {image|ublk} [learner].\n' >&2
    exit 1
fi
if [[ $(id -u) != 0 || ! -c /dev/fuse ||
      $(findmnt -n -o FSTYPE -T /dev/shm) != tmpfs ]]; then
    printf 'Use guest root with /dev/fuse, /dev/shm on tmpfs, and a private mount namespace.\n' >&2
    exit 1
fi
require_private_mount_namespace
require_private_mount_parent /dev/shm

build_dir=$(mktemp -d /dev/shm/io-journey-fuse-build.XXXXXX)
trap 'rm -rf -- "$build_dir"' EXIT
mkdir "$build_dir/tmp"
env CARGO_HOME="$build_dir/cargo" CARGO_TARGET_DIR="$build_dir/target" \
    TMPDIR="$build_dir/tmp" CARGO_INCREMENTAL=0 \
    cargo build --locked --quiet --manifest-path "$fuse_dir/Cargo.toml" "${build_args[@]}"

fuse_binary="$build_dir/target/debug/io-journey-fuse"
if [[ "$backend" == image ]]; then
    bash "$fuse_dir/workload.sh" "$fuse_binary"
else
    ublk_args=(fuse-case "$fuse_binary")
    if [[ "$course_mode" == learner ]]; then ublk_args+=(learner); fi
    bash "$fuse_dir/../ublk/smoke.sh" "${ublk_args[@]}"
fi
