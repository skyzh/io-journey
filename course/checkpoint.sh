#!/usr/bin/env bash
set -euo pipefail

repo_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)
baseline=bf555e462084f3406bc36d2aae7bf2a5d31686c2

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

retry_check() {
    printf 'After editing, retry from the course checkout with cargo xtask checkpoint check %q\n' "$lab_dir" >&2
    exit "$1"
}

[[ $# -eq 2 ]] || usage
if [[ "$1" == prepare ]]; then
    chapter=$2
    choose_tests "$chapter"
    if ! git -C "$repo_dir" cat-file -e "$baseline^{tree}" 2>/dev/null; then
        printf '%s\n' "Checkpoint baseline $baseline is missing." \
            'Use a full-history clone. In a shallow clone, run git fetch --unshallow and then retry.' >&2
        exit 1
    fi
    lab_dir=$(mktemp -d /var/tmp/io-journey-lab.XXXXXX)
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
    /var/tmp/io-journey-lab.*|/dev/shm/io-journey-lab.*) ;;
    *) usage ;;
esac
IFS= read -r chapter < "$lab_dir/.io-journey-checkpoint"
choose_tests "$chapter"

cd -- "$lab_dir"

case "$1" in
    previous) run_test "$predecessor" ;;
    check)
        run_test "$exercise" || retry_check "$?"
        if [[ "$chapter" == log ]]; then
            run_test tests::demo_recovers_the_acknowledged_prefix_across_both_cuts || retry_check "$?"
        fi
        ;;
    faults)
        run_test ftl::tests::a_torn_program_persists_the_exact_prefix_without_publishing_a_mapping
        run_test ftl::tests::a_torn_write_requires_an_erased_page_in_both_images
        ;;
    *) usage ;;
esac
