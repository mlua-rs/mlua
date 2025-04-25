use std::ffi::CStr;
use std::mem;
use std::os::raw::c_int;

use crate::error::Result;
use crate::function::Function;
use crate::state::Lua;

pub use require::{NavigateError, Require};

// Since Luau has some missing standard functions, we re-implement them here

impl Lua {
    /// Create a custom Luau `require` function using provided [`Require`] implementation to find
    /// and load modules.
    #[cfg(any(feature = "luau", doc))]
    #[cfg_attr(docsrs, doc(cfg(feature = "luau")))]
    pub fn create_require_function<R: Require + 'static>(&self, require: R) -> Result<Function> {
        unsafe extern "C-unwind" fn mlua_require(state: *mut ffi::lua_State) -> c_int {
            let mut ar: ffi::lua_Debug = mem::zeroed();
            if ffi::lua_getinfo(state, 1, cstr!("s"), &mut ar) == 0 {
                ffi::luaL_error(state, cstr!("require is not supported in this context"));
            }
            let top = ffi::lua_gettop(state);
            ffi::lua_pushvalue(state, ffi::lua_upvalueindex(2)); // the "proxy" require function
            ffi::lua_pushvalue(state, 1); // require path
            ffi::lua_pushstring(state, ar.source); // current file
            ffi::lua_call(state, 2, ffi::LUA_MULTRET);
            ffi::lua_gettop(state) - top
        }

        unsafe {
            self.exec_raw((), move |state| {
                let requirer_ptr = ffi::lua_newuserdata_t::<Box<dyn Require>>(state, Box::new(require));
                ffi::luarequire_pushproxyrequire(state, require::init_config, requirer_ptr as *mut _);
                ffi::lua_pushcclosured(state, mlua_require, cstr!("require"), 2);
            })
        }
    }

    pub(crate) unsafe fn configure_luau(&self) -> Result<()> {
        let globals = self.globals();

        globals.raw_set("collectgarbage", self.create_c_function(lua_collectgarbage)?)?;

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
            let res = ffi::lua_gc(state, ffi::LUA_GCSTEP, arg as _);
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

mod require;
