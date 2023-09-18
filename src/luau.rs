use std::prelude::v1::*;

use std::ffi::CStr;
use std::ffi::{c_float, c_int};
use std::string::String as StdString;

use crate::chunk::ChunkMode;
use crate::error::{Error, Result};
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

        // Set `_VERSION` global to include version number
        // The environment variable `LUAU_VERSION` set by the build script
        if let Some(version) = ffi::luau_version() {
            globals.raw_set("_VERSION", format!("Luau {version}"))?;
        }

        Ok(())
    }
}

unsafe extern "C-unwind" fn lua_collectgarbage(state: *mut ffi::lua_State) -> c_int {
    let option = ffi::luaL_optstring(state, 1, cstr!("collect"));
    let option = CStr::from_ptr(option);
    let arg = ffi::luaL_optinteger(state, 2, 0);
    match option.to_str() {
        Ok("collect") => {
            ffi::lua_gc(state, ffi::LUA_GCCOLLECT, 0);
            0
        }
        Ok("stop") => {
            ffi::lua_gc(state, ffi::LUA_GCSTOP, 0);
            0
        }
        Ok("restart") => {
            ffi::lua_gc(state, ffi::LUA_GCRESTART, 0);
            0
        }
        Ok("count") => {
            let kbytes = ffi::lua_gc(state, ffi::LUA_GCCOUNT, 0) as ffi::lua_Number;
            let kbytes_rem = ffi::lua_gc(state, ffi::LUA_GCCOUNTB, 0) as ffi::lua_Number;
            ffi::lua_pushnumber(state, kbytes + kbytes_rem / 1024.0);
            1
        }
        Ok("step") => {
            let res = ffi::lua_gc(state, ffi::LUA_GCSTEP, arg);
            ffi::lua_pushboolean(state, res);
            1
        }
        Ok("isrunning") => {
            let res = ffi::lua_gc(state, ffi::LUA_GCISRUNNING, 0);
            ffi::lua_pushboolean(state, res);
            1
        }
        _ => ffi::luaL_error(state, cstr!("collectgarbage called with invalid option")),
    }
}

fn lua_require(lua: &Lua, name: Option<StdString>) -> Result<Value> {
    let name = name.ok_or_else(|| Error::runtime("invalid module name"))?;

    // Find module in the cache
    let state = lua.state();
    let loaded = unsafe {
        let _sg = StackGuard::new(state);
        check_stack(state, 2)?;
        protect_lua!(state, 0, 1, fn(state) {
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

    let (mut source, mut source_name) = (None, String::new());
    for path in search_path.split(';') {
        let file_path = path.replacen('?', &name, 1);
        if let Ok(buf) = std::fs::read(&file_path) {
            source = Some(buf);
            source_name = file_path;
            break;
        }
    }
    let source = source.ok_or_else(|| Error::runtime(format!("cannot find '{name}'")))?;

    let value = lua
        .load(&source)
        .set_name(&format!("={source_name}"))
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
unsafe extern "C-unwind" fn lua_vector(state: *mut ffi::lua_State) -> c_int {
    let x = ffi::luaL_checknumber(state, 1) as c_float;
    let y = ffi::luaL_checknumber(state, 2) as c_float;
    let z = ffi::luaL_checknumber(state, 3) as c_float;
    #[cfg(feature = "luau-vector4")]
    let w = ffi::luaL_checknumber(state, 4) as c_float;

    #[cfg(not(feature = "luau-vector4"))]
    ffi::lua_pushvector(state, x, y, z);
    #[cfg(feature = "luau-vector4")]
    ffi::lua_pushvector(state, x, y, z, w);
    1
}
