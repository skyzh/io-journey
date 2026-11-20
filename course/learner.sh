#!/usr/bin/env bash
set -euo pipefail

usage() {
    printf '%s\n' 'Use ./course/learner.sh {previous|map|collect|check|admit|order|flush|dispatch|commit|replay|receipt|completions|ack}.' >&2
    exit 2
}
(( $# == 1 )) || usage
features=learner
manifest=Cargo.toml
cargo_args=(--offline --lib)
case "$1" in
    previous) test_args=(nand::tests::programming_requires_an_erased_page_and_erase_clears_the_whole_block -- --exact) ;;
    map) test_args=(ftl::tests::successful_programs_publish_the_new_mapping -- --exact) ;;
    collect) test_args=(ftl::tests::collection_copies_live_pages_before_erasing_and_recovery_keeps_them -- --exact) ;;
    check) test_args=(ftl::tests::) ;;
    admit)
        features=learner-queue
        test_args=(queue::tests::admission_returns_the_request_when_all_slots_are_occupied -- --exact)
        ;;
    order)
        features=learner-queue
        test_args=(queue::tests::accepted_requests_get_distinct_ordered_completions -- --exact)
        ;;
    flush)
        features=learner-queue
        test_args=(queue::tests::)
        ;;
    dispatch)
        features=learner-queue
        test_args=(dispatch::tests::)
        ;;
    commit)
        features=learner-journal
        test_args=(journal_fs::tests::file_sync_keeps_the_durable_name_and_directory_sync_publishes_rename -- --exact)
        ;;
    replay)
        features=learner-journal
        test_args=(journal_fs::tests::)
        ;;
    receipt)
        features=learner-append
        test_args=(record::tests::)
        ;;
    completions|ack)
        features=learner-append
        manifest=linux/append/Cargo.toml
        cargo_args=()
        if [[ "$1" == completions ]]; then
            test_args=(engine::tests::)
        else
            test_args=(-- --skip engine::tests::)
        fi
        ;;
    *) usage ;;
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
cargo test --locked --manifest-path "$repo_dir/$manifest" --features "$features" "${cargo_args[@]}" "${test_args[@]}"
