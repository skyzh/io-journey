#!/usr/bin/env bash
set -euo pipefail

repo_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)
baseline=e2c2d782ae036d6f67359b0cff8cabc2a6eba7dc

usage() {
    printf '%s\n' 'cargo xtask checkpoint prepare {ftl|queue|filesystem|log}' \
        'cargo xtask checkpoint {previous|check|faults} LAB_DIRECTORY' >&2
    exit 2
}

choose_tests() {
    case "$1" in
        ftl)
            predecessor=nand::tests::programming_requires_an_erased_page_and_erase_clears_the_whole_block
            exercise=ftl::tests::collection_copies_live_pages_before_erasing_and_recovery_keeps_them
            ;;
        queue)
            predecessor=ftl::tests::collection_copies_live_pages_before_erasing_and_recovery_keeps_them
            exercise=queue::tests::a_bounded_queue_flushes_every_child_and_excludes_the_later_range
            ;;
        filesystem)
            predecessor=queue::tests::a_bounded_queue_flushes_every_child_and_excludes_the_later_range
            exercise=filesystem::tests::append_crosses_block_boundaries_and_capacity_failure_preserves_the_file
            ;;
        log)
            predecessor=filesystem::tests::append_crosses_block_boundaries_and_capacity_failure_preserves_the_file
            exercise=app::tests::recovery_rejects_an_incomplete_or_corrupt_record
            ;;
        *) usage ;;
    esac
}

run_test() {
    cargo test --locked --offline "$1" -- --exact
}

[[ $# -eq 2 ]] || usage
if [[ "$(findmnt -n -o FSTYPE -T /dev/shm)" != tmpfs ]]; then
    printf '%s\n' 'Run the checkpoint commands on Linux with /dev/shm mounted as tmpfs.' >&2
    exit 1
fi

if [[ "$1" == prepare ]]; then
    chapter=$2
    choose_tests "$chapter"
    lab_dir=$(mktemp -d /dev/shm/io-journey-lab.XXXXXX)
    trap "rm -rf -- \"$lab_dir\"" EXIT
    git -C "$repo_dir" archive "$baseline" Cargo.toml Cargo.lock src | tar -x -C "$lab_dir"
    cd -- "$lab_dir"
    git apply --no-index "$repo_dir/course/starters/$chapter.patch"
    printf '%s\n' "$chapter" > .io-journey-checkpoint
    trap - EXIT
    printf '%s\n' "$lab_dir"
    exit 0
fi

lab_dir=$(cd -- "$2" && pwd -P)
case "$lab_dir" in
    /dev/shm/io-journey-lab.*) ;;
    *) usage ;;
esac
IFS= read -r chapter < "$lab_dir/.io-journey-checkpoint"
choose_tests "$chapter"

mkdir -p "$lab_dir/.scratch/tmp"
export CARGO_HOME="$lab_dir/.scratch/cargo"
export CARGO_TARGET_DIR="$lab_dir/.scratch/target"
export TMPDIR="$lab_dir/.scratch/tmp"
export CARGO_INCREMENTAL=0
cd -- "$lab_dir"

case "$1" in
    previous) run_test "$predecessor" ;;
    check)
        run_test "$exercise"
        if [[ "$chapter" == log ]]; then
            run_test tests::demo_recovers_the_acknowledged_prefix_across_both_cuts
        fi
        ;;
    faults)
        run_test ftl::tests::a_torn_program_persists_the_exact_prefix_without_publishing_a_mapping
        run_test ftl::tests::a_torn_write_requires_an_erased_page_in_both_images
        ;;
    *) usage ;;
esac
