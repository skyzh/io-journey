#!/usr/bin/env bash
set -euo pipefail

[[ $# == 1 && ( "$1" == timing || "$1" == heap ) ]] || {
    printf '%s\n' 'Use cargo xtask ftl-snapshots timing or heap.' >&2
    exit 2
}
course_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)
scratch=$(mktemp -d /var/tmp/io-journey-snapshots.XXXXXX)
trap 'rm -rf -- "$scratch"' EXIT
export TMPDIR="$scratch"
flags=(--edition=2021 -C opt-level=3 -C embed-bitcode=no -C codegen-units=16 -A dead_code)
if [[ "$1" == heap ]]; then flags+=(--cfg allocator_probe); fi
rustc "${flags[@]}" "$course_dir/measure-snapshots.rs" -o "$scratch/copy"
rustc "${flags[@]}" --cfg 'feature="shared-pages"' "$course_dir/measure-snapshots.rs" -o "$scratch/shared"
if [[ "$1" == heap ]]; then
    printf '%s\n' 'implementation,barrier,stage,heap_before,heap_peak,heap_after,page_allocs,page_frees,page_payload_bytes'
    "$scratch/copy" 1
    "$scratch/shared" 1
else
    printf '%s\n' 'sweep,implementation,repetition,barrier,kind,programmed,payload_bytes_per_clone,flush_ns,export_ns,write_flush_ns'
    "$scratch/copy" 1
    "$scratch/shared" 1
    "$scratch/shared" 2
    "$scratch/copy" 2
fi
