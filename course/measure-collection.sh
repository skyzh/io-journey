#!/usr/bin/env bash
set -euo pipefail

if [[ $(findmnt -n -o FSTYPE -T /dev/shm) != tmpfs ]]; then
    printf '%s\n' '/dev/shm must be tmpfs.' >&2
    exit 1
fi
course_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)
scratch=$(mktemp -d /dev/shm/io-journey-collection.XXXXXX)
trap 'rm -rf -- "$scratch"' EXIT
export TMPDIR="$scratch"
flags=(--edition=2021 -C opt-level=3 -C embed-bitcode=no -C codegen-units=16 -A dead_code)
rustc "${flags[@]}" "$course_dir/measure-collection.rs" -o "$scratch/scan"
rustc "${flags[@]}" --cfg 'feature="indexed-ftl"' "$course_dir/measure-collection.rs" -o "$scratch/indexed"
printf '%s\n' 'fixture,sweep,implementation,pair,position,collection,trigger_ns,page_slots,map_slots,summary_reads,free_entries,programs,erases'
for fixture in small large; do
    "$scratch/scan" 1 "$fixture"
    "$scratch/indexed" 1 "$fixture"
    "$scratch/indexed" 2 "$fixture"
    "$scratch/scan" 2 "$fixture"
done
