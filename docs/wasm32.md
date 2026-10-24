# Compiling for WebAssembly

Lua 5.1-5.5 and Luau can be compiled for WebAssembly. The WASI
builds use [WASI SDK](https://github.com/WebAssembly/wasi-sdk) to compile and link
the C/C++ code, and [Wasmtime](https://github.com/bytecodealliance/wasmtime) to run it.

| Rust target | Rust toolchain | Output |
| --- | --- | --- |
| `wasm32-wasip1` | stable | WASI Preview 1 module |
| `wasm32-wasip2` | stable | WASI Preview 2 component |
| `wasm32-wasip3` | nightly | WASI Preview 3 component |
| `wasm32-unknown-unknown` | stable | Module or component, linked with a WASI Preview 1 or 2 C runtime |

LuaJIT and Luau's JIT backend are not supported.

## Setup

Run the commands below from the mlua repository root.

Install WASI SDK, for example on Apple Silicon:

```bash
WASI_SDK_VERSION=34
WASI_SDK_ARCHIVE="wasi-sdk-${WASI_SDK_VERSION}.0-arm64-macos"
mkdir -p "$HOME/.local"
curl -fL "https://github.com/WebAssembly/wasi-sdk/releases/download/wasi-sdk-${WASI_SDK_VERSION}/${WASI_SDK_ARCHIVE}.tar.gz" \
    | tar -xz -C "$HOME/.local"
export WASI_SDK_PATH="$HOME/.local/$WASI_SDK_ARCHIVE"
```

Install Wasmtime:

```bash
brew install wasmtime
```

Install the Rust target(s):

```bash
rustup target add wasm32-wasip1 wasm32-wasip2 wasm32-unknown-unknown
```

The [setup script](../tests/wasm/setup.sh) selects the SDK compilers,
libraries, linker flags and Cargo runner. Source it in the same shell as the
Cargo commands. It also enables Wasm exceptions, which the Lua runtimes need
for error handling.

## WASI Preview 1 and 2

To compile and run the basic tests with Lua 5.5 on Preview 2:

```bash
source tests/wasm/setup.sh wasm32-wasip2
cargo test --manifest-path tests/wasm/Cargo.toml \
    --target wasm32-wasip2 --lib --features lua55
```

The test crate accepts `lua55`, `lua54`, `lua53`, `lua52`, `lua51` or `luau`.
It enables vendored Lua automatically.

For your own application, enable `vendored` for Lua.
Source the setup script using the path to your mlua checkout,
then build or run your application:

```bash
source /path/to/mlua/tests/wasm/setup.sh wasm32-wasip2
cargo build --target wasm32-wasip2
cargo run --target wasm32-wasip2
```

Cargo uses Wasmtime as its runner. To run the output directly:

```bash
wasmtime -W exceptions target/wasm32-wasip2/debug/your-app.wasm
```

## wasm32-unknown-unknown

This build uses Rust's `wasm32-unknown-unknown` target with a WASI C runtime.
The host must provide the matching WASI imports and support Wasm exceptions.
Loading the output in a browser requires those facilities too.

The [test crate](../tests/wasm/src/lib.rs) is a working example. Run it with
either C runtime:

```bash
./tests/wasm/run.sh lua55 wasm32-wasip1
./tests/wasm/run.sh lua55 wasm32-wasip2
```

Append `--release` to build and run with optimizations, or `--debug` for the default debug build.

With Preview 1, it builds a `cdylib` and invokes the exported `run_tests` function.
With Preview 2, it builds a Rust `staticlib`, then links it with a small
[C entry point](../tests/wasm/main.c) into a command component.

To adapt this to your project:

1. Use the `allocator` module from the test crate as your global allocator. Rust
   and WASI libc must share an allocator so they do not independently manage the
   same linear memory.
2. Export your entry point as an `extern "C"` function with `#[unsafe(no_mangle)]`.
3. Follow [run.sh](../tests/wasm/run.sh) for the build and link commands, replacing
   the crate path, output name and entry point with your own. Its call to
   `setup.sh` selects the C runtime separately from the Rust target.
