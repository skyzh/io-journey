#!/usr/bin/env bash
set -euo pipefail

if (( $# != 1 )); then
    printf '%s\n' 'Use ./course/learner.sh {previous|map|collect|check}.' >&2
    exit 2
fi
case "$1" in
    previous) test_args=(nand::tests::programming_requires_an_erased_page_and_erase_clears_the_whole_block -- --exact) ;;
    map) test_args=(ftl::tests::successful_programs_publish_the_new_mapping -- --exact) ;;
    collect) test_args=(ftl::tests::collection_copies_live_pages_before_erasing_and_recovery_keeps_them -- --exact) ;;
    check) test_args=(ftl::tests::) ;;
    *) printf '%s\n' 'Use ./course/learner.sh {previous|map|collect|check}.' >&2; exit 2 ;;
esac

if [[ $(findmnt -n -o FSTYPE -T /dev/shm) != tmpfs ]]; then
    printf '%s\n' '/dev/shm must be tmpfs.' >&2
    exit 1
fi
repo_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)
scratch=$(mktemp -d /dev/shm/io-journey-learner.XXXXXX)
trap 'rm -rf -- "$scratch"' EXIT
mkdir "$scratch/tmp"
export CARGO_HOME="$scratch/cargo" CARGO_TARGET_DIR="$scratch/target"
export TMPDIR="$scratch/tmp" CARGO_INCREMENTAL=0
cargo test --locked --offline --manifest-path "$repo_dir/Cargo.toml" --features learner --lib "${test_args[@]}"
