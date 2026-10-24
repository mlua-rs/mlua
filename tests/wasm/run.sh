#!/usr/bin/env bash
set -euo pipefail

lua="${1:-luau}"
c_target="${2:-wasm32-wasip1}"
if [[ $# -gt 3 || ! "$lua" =~ ^(luau|lua5[1-5])$ || ! "$c_target" =~ ^wasm32-wasip[12]$ || ! "${3:---debug}" =~ ^--(debug|release)$ ]]; then
    echo "Usage: $0 [lua55|lua54|lua53|lua52|lua51|luau] [wasm32-wasip1|wasm32-wasip2] [--debug|--release]" >&2
    exit 2
fi

cd "$(dirname "$0")/../.."
source tests/wasm/setup.sh wasm32-unknown-unknown "$c_target"
export CARGO_TARGET_DIR="$PWD/target/wasm-tests"
cargo_args=(--manifest-path tests/wasm/Cargo.toml --target wasm32-unknown-unknown --features "$lua")
profile=debug
if [[ "${3:-}" == --release ]]; then
    cargo_args+=(--release)
    profile=release
fi
wasm_output_dir="$CARGO_TARGET_DIR/wasm32-unknown-unknown/$profile"
wasm_output="$wasm_output_dir/mlua_wasm_tests.wasm"
if [[ "$c_target" == wasm32-wasip1 ]]; then
    cargo build "${cargo_args[@]}"
    "${wasm_runner[@]}" --invoke run_tests "$wasm_output"
else
    # The SDK links the bare Rust static library into a WASIp2 command component.
    wasm_output="${wasm_output%.wasm}.component.wasm"
    cargo rustc "${cargo_args[@]}" --lib --crate-type staticlib
    "$WASI_SDK_PATH/bin/clang" --target="$c_target" tests/wasm/main.c \
        "$wasm_output_dir/libmlua_wasm_tests.a" \
        -L"$wasm_lib/eh" -lc++ -lsetjmp -o "$wasm_output"
    "${wasm_runner[@]}" "$wasm_output"
fi
