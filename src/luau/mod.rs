use std::ffi::{CStr, CString};
use std::os::raw::c_int;
use std::ptr;

use crate::chunk::ChunkMode;
use crate::error::{Error, Result};
use crate::function::Function;
use crate::state::{callback_error_ext, ExtraData, Lua};
use crate::traits::{FromLuaMulti, IntoLua};
use crate::types::MaybeSend;

pub use heap_dump::HeapDump;
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

    /// Set the memory category for subsequent allocations from this Lua state.
    ///
    /// The category "main" is reserved for the default memory category.
    /// Maximum of 255 categories can be registered.
    /// The category is set per Lua thread (state) and affects all allocations made from that
    /// thread.
    ///
    /// Return error if too many categories are registered or if the category name is invalid.
    ///
    /// See [`Lua::heap_dump`] for tracking memory usage by category.
    #[cfg(any(feature = "luau", doc))]
    #[cfg_attr(docsrs, doc(cfg(feature = "luau")))]
    pub fn set_memory_category(&self, category: &str) -> Result<()> {
        let lua = self.lock();

        if category.contains(|c| !matches!(c, 'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_')) {
            return Err(Error::runtime("invalid memory category name"));
        }
        let cat_id = unsafe {
            let extra = ExtraData::get(lua.state());
            match ((*extra).mem_categories.iter().enumerate())
                .find(|&(_, name)| name.as_bytes() == category.as_bytes())
            {
                Some((id, _)) => id as u8,
                None => {
                    let new_id = (*extra).mem_categories.len() as u8;
                    if new_id == 255 {
                        return Err(Error::runtime("too many memory categories registered"));
                    }
                    (*extra).mem_categories.push(CString::new(category).unwrap());
                    new_id
                }
            }
        };
        unsafe { ffi::lua_setmemcat(lua.state(), cat_id as i32) };

        Ok(())
    }

    /// Dumps the current Lua VM heap state.
    ///
    /// The returned `HeapDump` can be used to analyze memory usage.
    /// It's recommended to call [`Lua::gc_collect`] before dumping the heap.
    #[cfg(any(feature = "luau", doc))]
    #[cfg_attr(docsrs, doc(cfg(feature = "luau")))]
    pub fn heap_dump(&self) -> Result<HeapDump> {
        let lua = self.lock();
        unsafe { heap_dump::HeapDump::new(lua.state()).ok_or_else(|| Error::runtime("unable to dump heap")) }
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

mod heap_dump;
mod json;
mod require;
