#!/usr/bin/env bash
set -euo pipefail

adapter_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)
scratch=$(mktemp -d /var/tmp/io-journey-build-env-test.XXXXXX)
trap 'rm -rf -- "$scratch"' EXIT
source "$adapter_dir/build-env.sh"

fail() { printf '%s\n' "$*" >&2; exit 1; }
ldconfig() {
    touch "$fixture/looked-up"
    printf '\t%s (libc6,AArch64) => /usr/lib/%s\n' "$soname" "$soname"
}
dpkg-query() { touch "$fixture/checked-fallback"; return 1; }
cargo() {
    [[ "$*" == "build --locked --quiet --manifest-path $adapter_dir/Cargo.toml" ]]
    printf '%s\n' "${LIBCLANG_PATH:-}" > "$fixture/cargo-path"
}

fixture="$scratch/cpp-only"
mkdir "$fixture"
soname=libclang-cpp.so.21.1
if (unset LIBCLANG_PATH; build_adapter "$adapter_dir" "$fixture/build") > "$fixture/log" 2>&1; then
    printf '%s\n' 'The C++ library was accepted by the build helper.' >&2
    exit 1
fi
[[ -e "$fixture/looked-up" && -e "$fixture/checked-fallback" && ! -e "$fixture/cargo-path" ]]
grep -F 'Set LIBCLANG_PATH' "$fixture/log" >/dev/null

for soname in libclang.so libclang.so.1 libclang-21.so.1 libclang-3.9.so; do
    fixture="$scratch/$soname"
    mkdir "$fixture"
    (unset LIBCLANG_PATH; build_adapter "$adapter_dir" "$fixture/build")
    [[ -e "$fixture/looked-up" && ! -e "$fixture/checked-fallback" && -e "$fixture/cargo-path" ]]
done

fixture="$scratch/explicit"
mkdir -p "$fixture/clang"
(export LIBCLANG_PATH="$fixture/clang"; build_adapter "$adapter_dir" "$fixture/build")
[[ ! -e "$fixture/looked-up" && ! -e "$fixture/checked-fallback" ]]
[[ $(cat "$fixture/cargo-path") == "$fixture/clang" ]]
printf '%s\n' 'Verified the C++-only fallback, libclang C matches, and explicit path.'
