# mlua
[![Build Status]][circleci] [![Latest Version]][crates.io] [![API Documentation]][docs.rs]

[Build Status]: https://img.shields.io/circleci/project/github/khvzak/mlua.svg
[circleci]: https://circleci.com/gh/khvzak/mlua
[Latest Version]: https://img.shields.io/crates/v/mlua.svg
[crates.io]: https://crates.io/crates/mlua
[API Documentation]: https://docs.rs/mlua/badge.svg
[docs.rs]: https://docs.rs/mlua

[Guided Tour](examples/guided_tour.rs)

A fork of [rlua 0.15](https://github.com/kyren/rlua/tree/0.15.3) which provides a high level
interface between Rust and Lua. Unlike `rlua` , `mlua` supports Lua 5.1 (including LuaJIT) and 5.3.
The `mlua` goal is to be an easy to use, practical and flexible API between Rust and Lua but
*__not__* always 100% safe due to the Lua VM nature. Also, `mlua` provides a way to write native lua
modules in Rust.

### Usage

#### standalone mode
Add to `Cargo.toml` :

``` toml
[dependencies]
mlua = "0.1"
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

#### module mode

Add to `Cargo.toml` :

``` toml
[lib]
crate-type = ["cdylib"]

[dependencies]
mlua = "0.1"
mlua_derive = "0.1"
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

And then (macos example):

``` sh
$ cargo build
$ ln -s ./target/debug/libmy_module.dylib ./my_module.so
$ lua5.3 -e 'require("my_module").hello("world")'
```

## Safety

One of the `mlua` goals is to provide *safe* API between Rust and Lua.
Every place where the Lua C API may trigger an error longjmp
in any way is protected by `lua_pcall` , and the user of the library is protected
from directly interacting with unsafe things like the Lua stack, and there is
overhead associated with this safety.

Unfortunately, `mlua` does not provide absolute safety even without using `unsafe` .
This library contains a huge amount of unsafe code. There are almost
certainly bugs still lurking in this library!  It is surprisingly, fiendishly
difficult to use the Lua C API without the potential for unsafety.

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

    panic!("must be never executed")
```

`mlua` should also be panic safe in another way as well, which is that any `Lua`
instances or handles remains usable after a user generated panic, and such
panics should not break internal invariants or leak Lua stack space. This is
mostly important to safely use `mlua` types in Drop impls, as you should not be
using panics for general error handling.

Below is a list of `mlua` behaviors that should be considered a bug.
If you encounter them, a bug report would be very welcome:

  + If your program panics with a message that contains the string "mlua internal error", this is a  bug.

  + The above is true even for the internal panic about running out of stack space!  There are a few ways to generate normal script errors by running out of stack, but if you encounter a *panic* based on running out of stack, this is a bug.

  + Lua C API errors are handled by lonjmp. All instances where the Lua C API would otherwise longjmp over calling stack frames should be guarded against, except in internal callbacks where this is intentional. If you detect that `mlua` is triggering a longjmp over your Rust stack frames, this is a bug!

  + If you detect that, after catching a panic or during a Drop triggered from a panic, a `Lua` or handle method is triggering other bugs or there is a Lua stack space leak, this is a bug. `mlua` instances are supposed to remain fully usable in the face of user generated panics. This guarantee does not extend to panics marked with "mlua internal error" simply because that is already indicative of a separate bug.

## License

This project is licensed under the [MIT license](LICENSE)

