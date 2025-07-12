# mlua
[![Build Status]][github-actions] [![Latest Version]][crates.io] [![API Documentation]][docs.rs] [![Coverage Status]][codecov.io] ![MSRV]

[Build Status]: https://github.com/mlua-rs/mlua/workflows/CI/badge.svg
[github-actions]: https://github.com/mlua-rs/mlua/actions
[Latest Version]: https://img.shields.io/crates/v/mlua.svg
[crates.io]: https://crates.io/crates/mlua
[API Documentation]: https://docs.rs/mlua/badge.svg
[docs.rs]: https://docs.rs/mlua
[Coverage Status]: https://codecov.io/gh/mlua-rs/mlua/branch/main/graph/badge.svg?token=99339FS1CG
[codecov.io]: https://codecov.io/gh/mlua-rs/mlua
[MSRV]: https://img.shields.io/badge/rust-1.79+-brightgreen.svg?&logo=rust

[Guided Tour] | [Benchmarks] | [FAQ]

[Guided Tour]: examples/guided_tour.rs
[Benchmarks]: https://github.com/khvzak/script-bench-rs
[FAQ]: FAQ.md

`mlua` is a set of bindings to the [Lua](https://www.lua.org) programming language for Rust with a goal to provide a
_safe_ (as much as possible), high level, easy to use, practical and flexible API.

Started as an `rlua` fork, `mlua` supports Lua 5.4, 5.3, 5.2, 5.1 (including LuaJIT) and [Luau] and allows writing native Lua modules in Rust as well as using Lua in a standalone mode.

`mlua` is tested on Windows/macOS/Linux including module mode in [GitHub Actions] on `x86_64` platforms and cross-compilation to `aarch64` (other targets are also supported).

WebAssembly (WASM) is supported through `wasm32-unknown-emscripten` target for all Lua/Luau versions excluding JIT.

[GitHub Actions]: https://github.com/mlua-rs/mlua/actions
[Luau]: https://luau.org

## Usage

### Feature flags

`mlua` uses feature flags to reduce the amount of dependencies and compiled code, and allow to choose only required set of features.
Below is a list of the available feature flags. By default `mlua` does not enable any features.

* `lua54`: enable Lua [5.4] support
* `lua53`: enable Lua [5.3] support
* `lua52`: enable Lua [5.2] support
* `lua51`: enable Lua [5.1] support
* `luajit`: enable [LuaJIT] support
* `luajit52`: enable [LuaJIT] support with partial compatibility with Lua 5.2
* `luau`: enable [Luau] support (auto vendored mode)
* `luau-jit`: enable [Luau] support with JIT backend.
* `luau-vector4`: enable [Luau] support with 4-dimensional vector.
* `vendored`: build static Lua(JIT) libraries from sources during `mlua` compilation using [lua-src] or [luajit-src]
* `module`: enable module mode (building loadable `cdylib` library for Lua)
* `async`: enable async/await support (any executor can be used, eg. [tokio] or [async-std])
* `send`: make `mlua::Lua: Send + Sync` (adds [`Send`] requirement to `mlua::Function` and `mlua::UserData`)
* `error-send`: make `mlua:Error: Send + Sync`
* `serde`: add serialization and deserialization support to `mlua` types using [serde]
* `macros`: enable procedural macros (such as `chunk!`)
* `anyhow`: enable `anyhow::Error` conversion into Lua
* `userdata-wrappers`: opt into `impl UserData` for `Rc<T>`/`Arc<T>`/`Rc<RefCell<T>>`/`Arc<Mutex<T>>` where `T: UserData`

[5.4]: https://www.lua.org/manual/5.4/manual.html
[5.3]: https://www.lua.org/manual/5.3/manual.html
[5.2]: https://www.lua.org/manual/5.2/manual.html
[5.1]: https://www.lua.org/manual/5.1/manual.html
[LuaJIT]: https://luajit.org/
[Luau]: https://github.com/luau-lang/luau
[lua-src]: https://github.com/mlua-rs/lua-src-rs
[luajit-src]: https://github.com/mlua-rs/luajit-src-rs
[tokio]: https://github.com/tokio-rs/tokio
[async-std]: https://github.com/async-rs/async-std
[`Send`]: https://doc.rust-lang.org/std/marker/trait.Send.html
[serde]: https://github.com/serde-rs/serde

### Async/await support

`mlua` supports async/await for all Lua versions including Luau.

This works using Lua [coroutines](https://www.lua.org/manual/5.3/manual.html#2.6) and requires running [Thread](https://docs.rs/mlua/latest/mlua/struct.Thread.html) along with enabling `feature = "async"` in `Cargo.toml`.

**Examples**:
- [HTTP Client](examples/async_http_client.rs)
- [HTTP Client (json)](examples/async_http_reqwest.rs)
- [HTTP Server](examples/async_http_server.rs)
- [TCP Server](examples/async_tcp_server.rs)


**shell command examples**:
```shell
# async http client (hyper)
cargo run --example async_http_client --features=lua54,async,macros

# async http client (reqwest)
cargo run --example async_http_reqwest --features=lua54,async,macros,serde

# async http server
cargo run --example async_http_server --features=lua54,async,macros,send
curl -v http://localhost:3000
```

### Serde support

With the `serde` feature flag enabled, `mlua` allows you to serialize/deserialize any type that implements [`serde::Serialize`] and [`serde::Deserialize`] into/from [`mlua::Value`]. In addition, `mlua` provides the [`serde::Serialize`] trait implementation for `mlua::Value` (including `UserData` support).

[Example](examples/serde.rs)

[`serde::Serialize`]: https://docs.serde.rs/serde/ser/trait.Serialize.html
[`serde::Deserialize`]: https://docs.serde.rs/serde/de/trait.Deserialize.html
[`mlua::Value`]: https://docs.rs/mlua/latest/mlua/enum.Value.html

### Compiling

You have to enable one of the features: `lua54`, `lua53`, `lua52`, `lua51`, `luajit(52)` or `luau`, according to the chosen Lua version.

By default `mlua` uses `pkg-config` to find Lua includes and libraries for the chosen Lua version.
In most cases it works as desired, although sometimes it may be preferable to use a custom Lua library.
To achieve this, mlua supports the `LUA_LIB`, `LUA_LIB_NAME` and `LUA_LINK` environment variables.
`LUA_LINK` is optional and may be `dylib` (a dynamic library) or `static` (a static library, `.a` archive).

An example of how to use them:
``` sh
my_project $ LUA_LIB=$HOME/tmp/lua-5.2.4/src LUA_LIB_NAME=lua LUA_LINK=static cargo build
```

`mlua` also supports vendored Lua/LuaJIT using the auxiliary crates [lua-src](https://crates.io/crates/lua-src) and
[luajit-src](https://crates.io/crates/luajit-src).
Just enable the `vendored` feature and cargo will automatically build and link the specified Lua/LuaJIT version. This is the easiest way to get started with `mlua`.

### Standalone mode
In standalone mode, `mlua` allows adding scripting support to your application with a gently configured Lua runtime to ensure safety and soundness.

Add to `Cargo.toml`:

``` toml
[dependencies]
mlua = { version = "0.11", features = ["lua54", "vendored"] }
```

`main.rs`

``` rust
use mlua::prelude::*;

fn main() -> LuaResult<()> {
    let lua = Lua::new();

    let map_table = lua.create_table()?;
    map_table.set(1, "one")?;
    map_table.set("two", 2)?;

    lua.globals().set("map_table", map_table)?;

    lua.load("for k,v in pairs(map_table) do print(k,v) end").exec()?;

    Ok(())
}
```

### Module mode
In module mode, `mlua` allows creating a compiled Lua module that can be loaded from Lua code using [`require`](https://www.lua.org/manual/5.4/manual.html#pdf-require). In this case `mlua` uses an external Lua runtime which could lead to potential unsafety due to the unpredictability of the Lua environment and usage of libraries such as [`debug`](https://www.lua.org/manual/5.4/manual.html#6.10).

[Example](examples/module)

Add to `Cargo.toml`:

``` toml
[lib]
crate-type = ["cdylib"]

[dependencies]
mlua = { version = "0.11", features = ["lua54", "module"] }
```

`lib.rs`:

``` rust
use mlua::prelude::*;

fn hello(_: &Lua, name: String) -> LuaResult<()> {
    println!("hello, {}!", name);
    Ok(())
}

#[mlua::lua_module]
fn my_module(lua: &Lua) -> LuaResult<LuaTable> {
    let exports = lua.create_table()?;
    exports.set("hello", lua.create_function(hello)?)?;
    Ok(exports)
}
```

And then (**macOS** example):

``` sh
$ cargo rustc -- -C link-arg=-undefined -C link-arg=dynamic_lookup
$ ln -s ./target/debug/libmy_module.dylib ./my_module.so
$ lua5.4 -e 'require("my_module").hello("world")'
hello, world!
```

On macOS, you need to set additional linker arguments. One option is to compile with `cargo rustc --release -- -C link-arg=-undefined -C link-arg=dynamic_lookup`, the other is to create a `.cargo/config.toml` with the following content:
``` toml
[target.x86_64-apple-darwin]
rustflags = [
  "-C", "link-arg=-undefined",
  "-C", "link-arg=dynamic_lookup",
]

[target.aarch64-apple-darwin]
rustflags = [
  "-C", "link-arg=-undefined",
  "-C", "link-arg=dynamic_lookup",
]
```
On Linux you can build modules normally with `cargo build --release`.

On Windows the target module will be linked with the `lua5x.dll` library (depending on your feature flags).
Your main application should provide this library.

Module builds don't require Lua binaries or headers to be installed on the system.

### Publishing to luarocks.org

There is a LuaRocks build backend for mlua modules: [`luarocks-build-rust-mlua`].

Modules written in Rust and published to luarocks:
- [`decasify`](https://github.com/alerque/decasify)
- [`lua-ryaml`](https://github.com/khvzak/lua-ryaml)
- [`tiktoken_core`](https://github.com/gptlang/lua-tiktoken)
- [`toml-edit`](https://github.com/vhyrro/toml-edit.lua)
- [`typst-lua`](https://github.com/rousbound/typst-lua)

[`luarocks-build-rust-mlua`]: https://luarocks.org/modules/khvzak/luarocks-build-rust-mlua

## Safety

One of `mlua`'s goals is to provide a *safe* API between Rust and Lua.
Every place where the Lua C API may trigger an error longjmp is protected by `lua_pcall`,
and the user of the library is protected from directly interacting with unsafe things like the Lua stack.
There is overhead associated with this safety.

Unfortunately, `mlua` does not provide absolute safety even without using `unsafe` .
This library contains a huge amount of unsafe code. There are almost certainly bugs still lurking in this library!
It is surprisingly, fiendishly difficult to use the Lua C API without the potential for unsafety.

## Panic handling

`mlua` wraps panics that are generated inside Rust callbacks in a regular Lua error. Panics can then be
resumed by returning or propagating the Lua error to Rust code.

For example:
``` rust
let lua = Lua::new();
let f = lua.create_function(|_, ()| -> LuaResult<()> {
    panic!("test panic");
})?;
lua.globals().set("rust_func", f)?;

let _ = lua.load(r#"
    local status, err = pcall(rust_func)
    print(err) -- prints: test panic
    error(err) -- propagate panic
"#).exec();

unreachable!()
```

Optionally, `mlua` can disable Rust panic catching in Lua via `pcall`/`xpcall` and automatically resume
them across the Lua API boundary. This is controlled via `LuaOptions` and done by wrapping the Lua `pcall`/`xpcall`
functions to prevent catching errors that are wrapped Rust panics.

`mlua` should also be panic safe in another way as well, which is that any `Lua` instances or handles
remain usable after a user generated panic, and such panics should not break internal invariants or
leak Lua stack space. This is mostly important to safely use `mlua` types in Drop impls, as you should not be
using panics for general error handling.

Below is a list of `mlua` behaviors that should be considered a bug.
If you encounter them, a bug report would be very welcome:

  + If you can cause UB with `mlua` without typing the word "unsafe", this is a bug.

  + If your program panics with a message that contains the string "mlua internal error", this is a bug.

  + Lua C API errors are handled by longjmp. All instances where the Lua C API would otherwise longjmp over calling stack frames should be guarded against, except in internal callbacks where this is intentional. If you detect that `mlua` is triggering a longjmp over your Rust stack frames, this is a bug!

  + If you detect that, after catching a panic or during a Drop triggered from a panic, a `Lua` or handle method is triggering other bugs or there is a Lua stack space leak, this is a bug. `mlua` instances are supposed to remain fully usable in the face of user generated panics. This guarantee does not extend to panics marked with "mlua internal error" simply because that is already indicative of a separate bug.

## Sandboxing

Please check the [Luau Sandboxing] page if you are interested in running untrusted Lua scripts in a controlled environment.

`mlua` provides the `Lua::sandbox` method for enabling sandbox mode (Luau only).

[Luau Sandboxing]: https://luau.org/sandbox

## License

This project is licensed under the [MIT license](LICENSE).
