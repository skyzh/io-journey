# Shared Cargo setup for the ext4 smoke and queue benchmark.
build_adapter() {
    local adapter_dir=$1 build_dir=$2
    shift 2
    mkdir -p "$build_dir/tmp"
    export CARGO_HOME="$build_dir/cargo"
    export CARGO_TARGET_DIR="$build_dir/target"
    export TMPDIR="$build_dir/tmp"
    export CARGO_INCREMENTAL=0
    if [[ -z "${LIBCLANG_PATH:-}" ]] && ! ldconfig -p | grep -F libclang >/dev/null; then
        [[ $(dpkg-query -W -f='${db:Status-Status}' libllvm21 2>/dev/null) == installed ]] ||
            fail 'Set LIBCLANG_PATH to an available libclang shared library directory.'
        mkdir "$build_dir/clang"
        chmod 0711 "$build_dir"
        install -d -m 0700 -o _apt "$build_dir/debs"
        (
            cd "$build_dir/debs"
            apt-get download libclang1-21
            for archive in libclang1-21_*.deb; do
                dpkg-deb -x "$archive" "$build_dir/clang"
            done
        )
        chmod 0700 "$build_dir"
        export LIBCLANG_PATH="$build_dir/clang/usr/lib/llvm-21/lib"
    fi
    cargo build --locked --quiet --manifest-path "$adapter_dir/Cargo.toml" "$@"
}
