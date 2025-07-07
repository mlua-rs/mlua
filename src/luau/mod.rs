use std::ffi::CStr;
use std::os::raw::c_int;
use std::ptr;

use crate::chunk::ChunkMode;
use crate::error::Result;
use crate::function::Function;
use crate::state::{callback_error_ext, ExtraData, Lua};
use crate::traits::{FromLuaMulti, IntoLua};
use crate::types::MaybeSend;

pub use require::{NavigateError, Require, TextRequirer};

// Since Luau has some missing standard functions, we re-implement them here

impl Lua {
    /// Create a custom Luau `require` function using provided [`Require`] implementation to find
    /// and load modules.
    #[cfg(any(feature = "luau", doc))]
    #[cfg_attr(docsrs, doc(cfg(feature = "luau")))]
    pub fn create_require_function<R: Require + MaybeSend + 'static>(&self, require: R) -> Result<Function> {
        require::create_require_function(self, require)
    }

    pub(crate) unsafe fn configure_luau(&self) -> Result<()> {
        let globals = self.globals();

        globals.raw_set("collectgarbage", self.create_c_function(lua_collectgarbage)?)?;
        globals.raw_set("loadstring", self.create_c_function(lua_loadstring)?)?;

        // Set `_VERSION` global to include version number
        // The environment variable `LUAU_VERSION` set by the build script
        if let Some(version) = ffi::luau_version() {
            globals.raw_set("_VERSION", format!("Luau {version}"))?;
        }

        // Enable default `require` implementation
        let require = self.create_require_function(require::TextRequirer::new())?;
        self.globals().raw_set("require", require)?;

        Ok(())
    }
}

unsafe extern "C-unwind" fn lua_collectgarbage(state: *mut ffi::lua_State) -> c_int {
    let option = ffi::luaL_optstring(state, 1, cstr!("collect"));
    let option = CStr::from_ptr(option);
    let arg = ffi::luaL_optinteger(state, 2, 0);
    let is_sandboxed = (*ExtraData::get(state)).sandboxed;
    match option.to_str() {
        Ok("collect") if !is_sandboxed => {
            ffi::lua_gc(state, ffi::LUA_GCCOLLECT, 0);
            0
        }
        Ok("stop") if !is_sandboxed => {
            ffi::lua_gc(state, ffi::LUA_GCSTOP, 0);
            0
        }
        Ok("restart") if !is_sandboxed => {
            ffi::lua_gc(state, ffi::LUA_GCRESTART, 0);
            0
        }
        Ok("count") => {
            let kbytes = ffi::lua_gc(state, ffi::LUA_GCCOUNT, 0) as ffi::lua_Number;
            let kbytes_rem = ffi::lua_gc(state, ffi::LUA_GCCOUNTB, 0) as ffi::lua_Number;
            ffi::lua_pushnumber(state, kbytes + kbytes_rem / 1024.0);
            1
        }
        Ok("step") if !is_sandboxed => {
            let res = ffi::lua_gc(state, ffi::LUA_GCSTEP, arg as _);
            ffi::lua_pushboolean(state, res);
            1
        }
        Ok("isrunning") if !is_sandboxed => {
            let res = ffi::lua_gc(state, ffi::LUA_GCISRUNNING, 0);
            ffi::lua_pushboolean(state, res);
            1
        }
        _ => ffi::luaL_error(state, cstr!("collectgarbage called with invalid option")),
    }
}

unsafe extern "C-unwind" fn lua_loadstring(state: *mut ffi::lua_State) -> c_int {
    callback_error_ext(state, ptr::null_mut(), false, move |extra, nargs| {
        let rawlua = (*extra).raw_lua();
        let (chunk, chunk_name) =
            <(String, Option<String>)>::from_stack_args(nargs, 1, Some("loadstring"), rawlua)?;
        let chunk_name = chunk_name.as_deref().unwrap_or("=(loadstring)");
        (rawlua.lua())
            .load(chunk)
            .set_name(chunk_name)
            .set_mode(ChunkMode::Text)
            .into_function()?
            .push_into_stack(rawlua)?;
        Ok(1)
    })
}

mod require;
