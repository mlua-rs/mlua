#!/usr/bin/env bash

# Source with a Rust target and optional C target for wasm32-unknown-unknown.
: "${WASI_SDK_PATH:?Set WASI_SDK_PATH}"
wasm_target="${1:?Pass a Rust target}"
wasm_c_target="$wasm_target"
if [[ "$wasm_target" == wasm32-unknown-unknown ]]; then
    wasm_c_target="${2:-wasm32-wasip1}"
fi
wasm_cc_target="${wasm_target//-/_}"
wasm_cargo_target="CARGO_TARGET_$(printf '%s' "$wasm_cc_target" | tr '[:lower:]' '[:upper:]')"
wasm_lib="$WASI_SDK_PATH/share/wasi-sysroot/lib/$wasm_c_target"
wasm_rustflags="-Lnative=$wasm_lib/eh -Lnative=$wasm_lib -lstatic=c++abi -lstatic=unwind -lstatic=wasi-emulated-process-clocks"
# Match the SDK's standard Wasm exception ABI.
# https://github.com/WebAssembly/wasi-sdk/blob/wasi-sdk-34/CppExceptions.md
wasm_cxxflags="-fexceptions -fwasm-exceptions -mllvm -wasm-use-legacy-eh=false"
wasm_runner=(wasmtime -W exceptions)
wasm_env=(
    "CC_$wasm_cc_target=$WASI_SDK_PATH/bin/clang"
    "CXX_$wasm_cc_target=$WASI_SDK_PATH/bin/clang++"
    "AR_$wasm_cc_target=$WASI_SDK_PATH/bin/llvm-ar"
    "CXXSTDLIB_$wasm_cc_target=c++"
)
if [[ "$wasm_target" == wasm32-unknown-unknown ]]; then
    wasm_env+=("CFLAGS_$wasm_cc_target=--target=$wasm_c_target")
    wasm_cxxflags="--target=$wasm_c_target $wasm_cxxflags"
    if [[ "$wasm_c_target" == wasm32-wasip1 ]]; then
        wasm_rustflags+=" -lstatic=c"
    fi
else
    wasm_env+=("${wasm_cargo_target}_LINKER=$WASI_SDK_PATH/bin/clang")
    wasm_rustflags+=" -Clink-self-contained=no"
    if [[ "$wasm_target" != wasm32-wasip1 ]]; then
        wasm_rustflags+=" -Clink-arg=-Wl,--export=cabi_realloc"
    fi
fi
if [[ "$wasm_target" == wasm32-wasip3 ]]; then
    wasm_runner+=(-S p3)
fi
wasm_env+=(
    "LUAU_CXXFLAGS=$wasm_cxxflags"
    "${wasm_cargo_target}_RUSTFLAGS=$wasm_rustflags"
    "${wasm_cargo_target}_RUNNER=${wasm_runner[*]}"
)
export "${wasm_env[@]}"
