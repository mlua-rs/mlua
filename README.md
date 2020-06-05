# mlua
[![Build Status]][github-actions] [![Latest Version]][crates.io] [![API Documentation]][docs.rs]

[Build Status]: https://github.com/khvzak/mlua/workflows/CI/badge.svg
[github-actions]: https://github.com/khvzak/mlua/actions
[Latest Version]: https://img.shields.io/crates/v/mlua.svg
[crates.io]: https://crates.io/crates/mlua
[API Documentation]: https://docs.rs/mlua/badge.svg
[docs.rs]: https://docs.rs/mlua

[Guided Tour](examples/guided_tour.rs)

`mlua` is bindings to [Lua](https://www.lua.org) programming language for Rust with a goal to provide
_safe_ (as far as it's possible), high level, easy to use, practical and flexible API.

Started as [rlua v0.15](https://github.com/amethyst/rlua/tree/0.15.3) fork, `mlua` supports *__all__* major Lua versions (including LuaJIT) and allows to write native Lua modules in Rust as well as use Lua in a standalone mode.

`mlua` supports the following Lua versions (and tested on Windows/macOS/Linux):
- Lua 5.4 (`feature = "lua54"`)
- Lua 5.3 (`feature = "lua53"`)
- Lua 5.2 (`feature = "lua52"`)
- Lua 5.1 (`feature = "lua51"`)
- LuaJIT 2.1.0 beta (`feature = "luajit"`)
- LuaJIT 2.0.5 stable (`feature = "luajit"`)

Additional `feature = "vendored"` enables building static Lua from sources during `mlua` compilation.

## Usage

### Async/await support

Starting from v0.3, `mlua` supports async/await for all Lua versions. This works using Lua [coroutines](https://www.lua.org/manual/5.3/manual.html#2.6) and require running [Thread](https://docs.rs/mlua/latest/mlua/struct.Thread.html) along with enabling `feature = "async"` in `Cargo.toml`.

**Examples**:
- [HTTP Client](examples/async_http_client.rs)
- [HTTP Server](examples/async_http_server.rs)
- [TCP Server](examples/async_tcp_server.rs)

### Compiling

You have to enable one of the features `lua54`, `lua53`, `lua52`, `lua51` or `luajit`, according to the choosen Lua version.

By default `mlua` uses `pkg-config` tool to find lua includes and libraries for the chosen Lua version.
In most cases it works as desired, although sometimes could be more preferable to use a custom lua library.
To achieve this, mlua supports `LUA_INC`, `LUA_LIB`, `LUA_LIB_NAME` and `LUA_LINK` environment variables.
`LUA_LINK` is optional and may be `dylib` (a dynamic library) or `static` (a static library, `.a` archive).

An example how to use them:
``` sh
my_project $ LUA_INC=$HOME/tmp/lua-5.2.4/src LUA_LIB=$HOME/tmp/lua-5.2.4/src LUA_LIB_NAME=lua LUA_LINK=static cargo build
```

`mlua` also supports vendored lua/luajit using the auxilary crates [lua-src](https://crates.io/crates/lua-src) and
[luajit-src](https://crates.io/crates/luajit-src).
Just enable the `vendored` feature and cargo will automatically build and link specified lua/luajit version. This is the easiest way to get started with `mlua`.

### Standalone mode
Add to `Cargo.toml` :

``` toml
[dependencies]
mlua = { version = "0.4", features = ["lua53"] }
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

[Example](examples/module)

Add to `Cargo.toml` :

``` toml
[lib]
crate-type = ["cdylib"]

[dependencies]
mlua = { version = "0.4", features = ["lua53", "module"] }
mlua_derive = "0.4"
```

`lib.rs` :

``` rust
#[macro_use]
extern crate mlua_derive;
use mlua::prelude::*;

fn hello(_: &Lua, name: String) -> LuaResult<()> {
    println!("hello, {}!", name);
    Ok(())
}

#[lua_module]
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
$ lua5.3 -e 'require("my_module").hello("world")'
hello, world!
```

On macOS, you need to set additional linker arguments. One option is to compile with `cargo rustc --release -- -C link-arg=-undefined -C link-arg=dynamic_lookup`, the other is to create a `.cargo/config` with the following content:
``` toml
[target.x86_64-apple-darwin]
rustflags = [
  "-C", "link-arg=-undefined",
  "-C", "link-arg=dynamic_lookup",
]
```
On Linux you can build modules normally with `cargo build --release`.
Vendored and non-vendored builds are supported for these OS.

On Windows `vendored` mode is not supported since you need to link to a Lua dll.
Easiest way is to use either MinGW64 (as part of [MSYS2](https://github.com/msys2/msys2) package) with `pkg-config` or
MSVC with `LUA_INC` / `LUA_LIB` / `LUA_LIB_NAME` environment variables.

More details about compiling and linking Lua modules can be found on the [Building Modules](http://lua-users.org/wiki/BuildingModules) page.

## Safety

One of the `mlua` goals is to provide *safe* API between Rust and Lua.
Every place where the Lua C API may trigger an error longjmp in any way is protected by `lua_pcall`,
and the user of the library is protected from directly interacting with unsafe things like the Lua stack,
and there is overhead associated with this safety.

Unfortunately, `mlua` does not provide absolute safety even without using `unsafe` .
This library contains a huge amount of unsafe code. There are almost certainly bugs still lurking in this library!
It is surprisingly, fiendishly difficult to use the Lua C API without the potential for unsafety.

## Panic handling

`mlua` wraps panics that are generated inside Rust callbacks in a regular Lua error. Panics could be
resumed then by propagating the Lua error to Rust code.

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

`mlua` should also be panic safe in another way as well, which is that any `Lua` instances or handles
remains usable after a user generated panic, and such panics should not break internal invariants or
leak Lua stack space. This is mostly important to safely use `mlua` types in Drop impls, as you should not be
using panics for general error handling.

Below is a list of `mlua` behaviors that should be considered a bug.
If you encounter them, a bug report would be very welcome:

  + If your program panics with a message that contains the string "mlua internal error", this is a  bug.

  + The above is true even for the internal panic about running out of stack space!  There are a few ways to generate normal script errors by running out of stack, but if you encounter a *panic* based on running out of stack, this is a bug.

  + Lua C API errors are handled by lonjmp. All instances where the Lua C API would otherwise longjmp over calling stack frames should be guarded against, except in internal callbacks where this is intentional. If you detect that `mlua` is triggering a longjmp over your Rust stack frames, this is a bug!

  + If you detect that, after catching a panic or during a Drop triggered from a panic, a `Lua` or handle method is triggering other bugs or there is a Lua stack space leak, this is a bug. `mlua` instances are supposed to remain fully usable in the face of user generated panics. This guarantee does not extend to panics marked with "mlua internal error" simply because that is already indicative of a separate bug.

## License

This project is licensed under the [MIT license](LICENSE)

