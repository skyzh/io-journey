#!/usr/bin/env bash
set -euo pipefail

usage() {
    printf '%s\n' 'Use cargo xtask learner {nand|map|collect|check|admit|order|flush|dispatch|scheduler|cache|commit|replay|receipt|completions|ack}.' >&2
    exit 2
}
(( $# == 1 )) || usage
case "$1" in
    all)
        stages=(nand map collect check admit order flush dispatch scheduler cache commit replay receipt completions ack)
        retry='cargo xtask test'
        ;;
    nand|map|collect|check|admit|order|flush|dispatch|scheduler|cache|commit|replay|receipt|completions|ack)
        stages=("$1")
        retry="cargo xtask learner $1"
        ;;
    *) usage ;;
esac

repo_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)

run_stage() {
    local stage=$1 features=learner label
    local -a test_args=(--)
    case "$stage" in
        nand)
            label='NAND to SSD · Read, program, and erase NAND'
            features=learner-nand
            test_args=(nand::tests::)
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
            label='Blocks in flight · Choose a device request'
            features=learner-queue
            test_args=(dispatch::tests::)
            ;;
        scheduler)
            label='Block scheduling · Dispatch without crossing dependencies'
            features=learner-scheduler
            test_args=(scheduler::tests::)
            ;;
        cache)
            label='Blocks to files · Write back buffered blocks'
            features=learner-cache
            test_args=(cache::tests::)
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
            label='Efficient application I/O · Retire a bounded mock wave'
            features=learner-append
            test_args=(ring::tests::)
            ;;
        ack)
            label='Efficient application I/O · Acknowledge after sync'
            features=learner-append
            test_args=(ring::commit_tests::)
            ;;
    esac

    printf '\n%s\n' "$label"
    cargo test --locked --offline --lib --manifest-path "$repo_dir/Cargo.toml" --features "$features" "${test_args[@]}" || {
        local status=$?
        printf 'Retry from the course checkout with %s\n' "$retry" >&2
        return "$status"
    }
}

for stage in "${stages[@]}"; do
    run_stage "$stage"
done
