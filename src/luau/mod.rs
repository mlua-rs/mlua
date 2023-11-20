use std::ffi::CStr;
use std::os::raw::{c_float, c_int};

use crate::error::Result;
use crate::lua::Lua;

// Since Luau has some missing standard functions, we re-implement them here

impl Lua {
    pub(crate) unsafe fn configure_luau(&self) -> Result<()> {
        let globals = self.globals();

        globals.raw_set(
            "collectgarbage",
            self.create_c_function(lua_collectgarbage)?,
        )?;
        globals.raw_set("vector", self.create_c_function(lua_vector)?)?;

        // Set `_VERSION` global to include version number
        // The environment variable `LUAU_VERSION` set by the build script
        if let Some(version) = ffi::luau_version() {
            globals.raw_set("_VERSION", format!("Luau {version}"))?;
        }

        Ok(())
    }

    pub(crate) fn disable_c_modules(&self) -> Result<()> {
        package::disable_dylibs(self);
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

pub(crate) use package::register_package_module;

mod package;
