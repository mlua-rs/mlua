Registers Lua module entrypoint.

You can register multiple entrypoints as required.

```rust,ignore
use mlua::{Lua, Result, Table};

#[mlua::lua_module]
fn my_module(lua: &Lua) -> Result<Table> {
    let exports = lua.create_table()?;
    exports.set("hello", "world")?;
    Ok(exports)
}
```

Internally in the code above the compiler defines C function `luaopen_my_module`.

You can also pass options to the attribute:

* name - name of the module, defaults to the name of the function

```rust,ignore
#[mlua::lua_module(name = "alt_module")]
fn my_module(lua: &Lua) -> Result<Table> {
    ...
}
```

* skip_memory_check - skip memory allocation checks for some operations.

In module mode, mlua runs in an unknown environment and cannot tell whether there are any memory
limits or not. As a result, some operations that require memory allocation run in protected
mode. Setting this attribute will improve performance of such operations with risk of having
uncaught exceptions and memory leaks.

```rust,ignore
#[mlua::lua_module(skip_memory_check)]
fn my_module(lua: &Lua) -> Result<Table> {
    ...
}
```
