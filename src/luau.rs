use std::ffi::CStr;
use std::os::raw::{c_float, c_int};

use crate::chunk::ChunkMode;
use crate::error::{Error, Result};
use crate::ffi;
use crate::lua::Lua;
use crate::table::Table;
use crate::util::{check_stack, StackGuard};
use crate::value::Value;

// Since Luau has some missing standard function, we re-implement them here

impl Lua {
    pub(crate) unsafe fn prepare_luau_state(&self) -> Result<()> {
        let globals = self.globals();

        globals.raw_set(
            "collectgarbage",
            self.create_c_function(lua_collectgarbage)?,
        )?;
        globals.raw_set("require", self.create_function(lua_require)?)?;
        globals.raw_set("vector", self.create_c_function(lua_vector)?)?;

        Ok(())
    }
}

unsafe extern "C" fn lua_collectgarbage(state: *mut ffi::lua_State) -> c_int {
    let option = ffi::luaL_optstring(state, 1, cstr!("collect"));
    let option = CStr::from_ptr(option);
    match option.to_str() {
        Ok("collect") => {
            ffi::lua_gc(state, ffi::LUA_GCCOLLECT, 0);
            0
        }
        Ok("count") => {
            let n = ffi::lua_gc(state, ffi::LUA_GCCOUNT, 0);
            ffi::lua_pushnumber(state, n as ffi::lua_Number);
            1
        }
        // TODO: More variants
        _ => ffi::luaL_error(
            state,
            cstr!("collectgarbage must be called with 'count' or 'collect'"),
        ),
    }
}

fn lua_require(lua: &Lua, name: Option<std::string::String>) -> Result<Value> {
    let name = name.ok_or_else(|| Error::RuntimeError("name is nil".into()))?;

    // Find module in the cache
    let loaded = unsafe {
        let _sg = StackGuard::new(lua.state);
        check_stack(lua.state, 2)?;
        protect_lua!(lua.state, 0, 1, fn(state) {
            ffi::luaL_getsubtable(state, ffi::LUA_REGISTRYINDEX, cstr!("_LOADED"));
        })?;
        Table(lua.pop_ref())
    };
    if let Some(v) = loaded.raw_get(name.clone())? {
        return Ok(v);
    }

    // Load file from filesystem
    let mut search_path = std::env::var("LUAU_PATH").unwrap_or_default();
    if search_path.is_empty() {
        search_path = "?.luau;?.lua".into();
    }

    let mut source = None;
    for path in search_path.split(';') {
        if let Ok(buf) = std::fs::read(path.replacen('?', &name, 1)) {
            source = Some(buf);
            break;
        }
    }
    let source = source.ok_or_else(|| Error::RuntimeError(format!("cannot find '{}'", name)))?;

    let value = lua
        .load(&source)
        .set_name(&format!("={}", name))?
        .set_mode(ChunkMode::Text)
        .call::<_, Value>(())?;

    // Save in the cache
    loaded.raw_set(
        name,
        match value.clone() {
            Value::Nil => Value::Boolean(true),
            v => v,
        },
    )?;

    Ok(value)
}

// Luau vector datatype constructor
unsafe extern "C" fn lua_vector(state: *mut ffi::lua_State) -> c_int {
    let x = ffi::luaL_checknumber(state, 1) as c_float;
    let y = ffi::luaL_checknumber(state, 2) as c_float;
    let z = ffi::luaL_checknumber(state, 3) as c_float;
    ffi::lua_pushvector(state, x, y, z);
    1
}
