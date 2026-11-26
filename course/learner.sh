#!/usr/bin/env bash
set -euo pipefail

usage() {
    printf '%s\n' 'Use cargo xtask learner {nand|previous|map|collect|check|index|sharing|admit|order|flush|dispatch|commit|replay|receipt|completions|ack}.' >&2
    exit 2
}
(( $# == 1 )) || usage
case "$1" in
    all)
        stages=(nand map collect check admit order flush dispatch commit replay receipt completions ack)
        retry='cargo xtask test'
        ;;
    nand|previous|map|collect|check|index|sharing|admit|order|flush|dispatch|commit|replay|receipt|completions|ack)
        stages=("$1")
        retry="cargo xtask learner $1"
        ;;
    *) usage ;;
esac

repo_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)

run_stage() {
    local stage=$1 features=learner manifest=Cargo.toml label
    local -a cargo_args=(--offline --lib) test_args=(--)
    case "$stage" in
        nand)
            label='NAND to SSD · Read, program, and erase NAND'
            features=learner-nand
            test_args=(nand::tests::)
            ;;
        previous)
            features=
            label='Supplied NAND programming and erasure'
            test_args=(nand::tests::programming_requires_an_erased_page_and_erase_clears_the_whole_block -- --exact)
            ;;
        map)
            label='NAND to SSD · Publish the mapping'
            test_args=(ftl::tests::successful_programs_publish_the_new_mapping -- --exact)
            ;;
        collect)
            label='NAND to SSD · Move live pages'
            test_args=(ftl::tests::collection_copies_live_pages_before_erasing_and_recovery_keeps_them -- --exact)
            ;;
        check)
            label='NAND to SSD · FTL and recovery'
            test_args=(ftl::tests::)
            ;;
        index)
            label='FTL free-space index'
            features=learner-index
            test_args=(ftl::tests::index_matches_physical_pages_and_mappings_through_recovery -- --exact)
            ;;
        sharing)
            label='Shared checkpoint pages'
            features=learner-sharing
            ;;
        admit)
            label='Blocks in flight · Admit requests'
            features=learner-queue
            test_args=(queue::tests::admission_returns_the_request_when_all_slots_are_occupied -- --exact)
            ;;
        order)
            label='Blocks in flight · Preserve order and IDs'
            features=learner-queue
            test_args=(queue::tests::accepted_requests_get_distinct_ordered_completions -- --exact)
            ;;
        flush)
            label='Blocks in flight · Flush and queue checks'
            features=learner-queue
            test_args=(queue::tests::)
            ;;
        dispatch)
            label='Blocks in flight · Choose a Linux tag'
            features=learner-queue
            test_args=(dispatch::tests::)
            ;;
        commit)
            label='Blocks to files · Publish a redo commit'
            features=learner-journal
            test_args=(journal_fs::tests::file_sync_keeps_the_durable_name_and_directory_sync_publishes_rename -- --exact)
            ;;
        replay)
            label='Blocks to files · Recover the journal'
            features=learner-journal
            test_args=(journal_fs::tests::)
            ;;
        receipt)
            label='Efficient application I/O · Recover the receipt'
            features=learner-append
            test_args=(record::tests::)
            ;;
        completions)
            label='Efficient application I/O · Retire a bounded wave'
            features=learner-append
            manifest=linux/append/Cargo.toml
            cargo_args=()
            test_args=(engine::tests::)
            ;;
        ack)
            label='Efficient application I/O · Acknowledge after sync'
            features=learner-append
            manifest=linux/append/Cargo.toml
            cargo_args=()
            test_args=(-- --skip engine::tests::)
            ;;
    esac

    if [[ "$manifest" == linux/* && $(uname -s) != Linux ]]; then
        printf 'The %s check uses Linux io_uring. From the checkout in a Linux shell, rerun %s\n' \
            "$stage" "$retry" >&2
        return 1
    fi

    printf '\n%s\n' "$label"
    cargo test --locked --manifest-path "$repo_dir/$manifest" --features "$features" "${cargo_args[@]}" "${test_args[@]}" || {
        local status=$?
        printf 'Retry from the course checkout with %s\n' "$retry" >&2
        return "$status"
    }
}

for stage in "${stages[@]}"; do
    run_stage "$stage"
done
