use std::cell::RefCell;
use std::ffi::CStr;
use std::io::Result as IoResult;
use std::ops::{Deref, DerefMut};
use std::os::raw::{c_char, c_int, c_void};
use std::result::Result as StdResult;
use std::{fmt, mem, ptr};

use crate::error::{Error, Result};
use crate::function::Function;
use crate::state::{callback_error_ext, Lua};
use crate::table::Table;
use crate::types::MaybeSend;

// TODO: Rename to FsRequirer
pub use fs::TextRequirer;

/// An error that can occur during navigation in the Luau `require-by-string` system.
#[derive(Debug, Clone)]
pub enum NavigateError {
    Ambiguous,
    NotFound,
    Other(Error),
}

#[cfg(feature = "luau")]
trait IntoNavigateResult {
    fn into_nav_result(self) -> Result<ffi::luarequire_NavigateResult>;
}

#[cfg(feature = "luau")]
impl IntoNavigateResult for StdResult<(), NavigateError> {
    fn into_nav_result(self) -> Result<ffi::luarequire_NavigateResult> {
        match self {
            Ok(()) => Ok(ffi::luarequire_NavigateResult::Success),
            Err(NavigateError::Ambiguous) => Ok(ffi::luarequire_NavigateResult::Ambiguous),
            Err(NavigateError::NotFound) => Ok(ffi::luarequire_NavigateResult::NotFound),
            Err(NavigateError::Other(err)) => Err(err),
        }
    }
}

impl From<Error> for NavigateError {
    fn from(err: Error) -> Self {
        NavigateError::Other(err)
    }
}

#[cfg(feature = "luau")]
type WriteResult = ffi::luarequire_WriteResult;

#[cfg(feature = "luau")]
type ConfigStatus = ffi::luarequire_ConfigStatus;

/// A trait for handling modules loading and navigation in the Luau `require-by-string` system.
pub trait Require {
    /// Returns `true` if "require" is permitted for the given chunk name.
    fn is_require_allowed(&self, chunk_name: &str) -> bool;

    /// Resets the internal state to point at the requirer module.
    fn reset(&mut self, chunk_name: &str) -> StdResult<(), NavigateError>;

    /// Resets the internal state to point at an aliased module.
    ///
    /// This function received an exact path from a configuration file.
    /// It's only called when an alias's path cannot be resolved relative to its
    /// configuration file.
    fn jump_to_alias(&mut self, path: &str) -> StdResult<(), NavigateError>;

    // Navigate to parent directory
    fn to_parent(&mut self) -> StdResult<(), NavigateError>;

    /// Navigate to the given child directory.
    fn to_child(&mut self, name: &str) -> StdResult<(), NavigateError>;

    /// Returns whether the context is currently pointing at a module.
    fn has_module(&self) -> bool;

    /// Provides a cache key representing the current module.
    ///
    /// This function is only called if `has_module` returns true.
    fn cache_key(&self) -> String;

    /// Returns whether a configuration is present in the current context.
    fn has_config(&self) -> bool;

    /// Returns the contents of the configuration file in the current context.
    ///
    /// This function is only called if `has_config` returns true.
    fn config(&self) -> IoResult<Vec<u8>>;

    /// Returns a loader function for the current module, that when called, loads the module
    /// and returns the result.
    ///
    /// Loader can be sync or async.
    /// This function is only called if `has_module` returns true.
    fn loader(&self, lua: &Lua) -> Result<Function>;
}

impl fmt::Debug for dyn Require {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "<dyn Require>")
    }
}

struct Context {
    require: Box<dyn Require>,
    config_cache: Option<IoResult<Vec<u8>>>,
}

impl Deref for Context {
    type Target = dyn Require;

    fn deref(&self) -> &Self::Target {
        &*self.require
    }
}

impl DerefMut for Context {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut *self.require
    }
}

impl Context {
    fn new(require: impl Require + MaybeSend + 'static) -> Self {
        Context {
            require: Box::new(require),
            config_cache: None,
        }
    }
}

macro_rules! try_borrow {
    ($state:expr, $ctx:expr) => {
        match (*($ctx as *const RefCell<Context>)).try_borrow() {
            Ok(ctx) => ctx,
            Err(_) => ffi::luaL_error($state, cstr!("require context is already borrowed")),
        }
    };
}

macro_rules! try_borrow_mut {
    ($state:expr, $ctx:expr) => {
        match (*($ctx as *const RefCell<Context>)).try_borrow_mut() {
            Ok(ctx) => ctx,
            Err(_) => ffi::luaL_error($state, cstr!("require context is already borrowed")),
        }
    };
}

#[cfg(feature = "luau")]
pub(super) unsafe extern "C-unwind" fn init_config(config: *mut ffi::luarequire_Configuration) {
    if config.is_null() {
        return;
    }

    unsafe extern "C-unwind" fn is_require_allowed(
        state: *mut ffi::lua_State,
        ctx: *mut c_void,
        requirer_chunkname: *const c_char,
    ) -> bool {
        if requirer_chunkname.is_null() {
            return false;
        }

        let this = try_borrow!(state, ctx);
        let chunk_name = CStr::from_ptr(requirer_chunkname).to_string_lossy();
        this.is_require_allowed(&chunk_name)
    }

    unsafe extern "C-unwind" fn reset(
        state: *mut ffi::lua_State,
        ctx: *mut c_void,
        requirer_chunkname: *const c_char,
    ) -> ffi::luarequire_NavigateResult {
        let mut this = try_borrow_mut!(state, ctx);
        let chunk_name = CStr::from_ptr(requirer_chunkname).to_string_lossy();
        callback_error_ext(state, ptr::null_mut(), true, move |_, _| {
            this.reset(&chunk_name).into_nav_result()
        })
    }

    unsafe extern "C-unwind" fn jump_to_alias(
        state: *mut ffi::lua_State,
        ctx: *mut c_void,
        path: *const c_char,
    ) -> ffi::luarequire_NavigateResult {
        let mut this = try_borrow_mut!(state, ctx);
        let path = CStr::from_ptr(path).to_string_lossy();
        callback_error_ext(state, ptr::null_mut(), true, move |_, _| {
            this.jump_to_alias(&path).into_nav_result()
        })
    }

    unsafe extern "C-unwind" fn to_parent(
        state: *mut ffi::lua_State,
        ctx: *mut c_void,
    ) -> ffi::luarequire_NavigateResult {
        let mut this = try_borrow_mut!(state, ctx);
        callback_error_ext(state, ptr::null_mut(), true, move |_, _| {
            this.to_parent().into_nav_result()
        })
    }

    unsafe extern "C-unwind" fn to_child(
        state: *mut ffi::lua_State,
        ctx: *mut c_void,
        name: *const c_char,
    ) -> ffi::luarequire_NavigateResult {
        let mut this = try_borrow_mut!(state, ctx);
        let name = CStr::from_ptr(name).to_string_lossy();
        callback_error_ext(state, ptr::null_mut(), true, move |_, _| {
            this.to_child(&name).into_nav_result()
        })
    }

    unsafe extern "C-unwind" fn is_module_present(state: *mut ffi::lua_State, ctx: *mut c_void) -> bool {
        let this = try_borrow!(state, ctx);
        this.has_module()
    }

    unsafe extern "C-unwind" fn get_chunkname(
        _state: *mut ffi::lua_State,
        _ctx: *mut c_void,
        buffer: *mut c_char,
        buffer_size: usize,
        size_out: *mut usize,
    ) -> WriteResult {
        write_to_buffer(buffer, buffer_size, size_out, &[])
    }

    unsafe extern "C-unwind" fn get_loadname(
        _state: *mut ffi::lua_State,
        _ctx: *mut c_void,
        buffer: *mut c_char,
        buffer_size: usize,
        size_out: *mut usize,
    ) -> WriteResult {
        write_to_buffer(buffer, buffer_size, size_out, &[])
    }

    unsafe extern "C-unwind" fn get_cache_key(
        state: *mut ffi::lua_State,
        ctx: *mut c_void,
        buffer: *mut c_char,
        buffer_size: usize,
        size_out: *mut usize,
    ) -> WriteResult {
        let this = try_borrow!(state, ctx);
        let cache_key = this.cache_key();
        write_to_buffer(buffer, buffer_size, size_out, cache_key.as_bytes())
    }

    unsafe extern "C-unwind" fn get_config_status(
        state: *mut ffi::lua_State,
        ctx: *mut c_void,
    ) -> ConfigStatus {
        let mut this = try_borrow_mut!(state, ctx);
        if this.has_config() {
            this.config_cache = Some(this.config());
            if let Some(Ok(data)) = &this.config_cache {
                return detect_config_format(data);
            }
        }
        ConfigStatus::Absent
    }

    unsafe extern "C-unwind" fn get_config(
        state: *mut ffi::lua_State,
        ctx: *mut c_void,
        buffer: *mut c_char,
        buffer_size: usize,
        size_out: *mut usize,
    ) -> WriteResult {
        let mut this = try_borrow_mut!(state, ctx);
        let config = callback_error_ext(state, ptr::null_mut(), true, move |_, _| {
            Ok(this.config_cache.take().unwrap_or_else(|| this.config())?)
        });
        write_to_buffer(buffer, buffer_size, size_out, &config)
    }

    unsafe extern "C-unwind" fn load(
        state: *mut ffi::lua_State,
        ctx: *mut c_void,
        _path: *const c_char,
        _chunkname: *const c_char,
        _loadname: *const c_char,
    ) -> c_int {
        let this = try_borrow!(state, ctx);
        callback_error_ext(state, ptr::null_mut(), true, move |extra, _| {
            let rawlua = (*extra).raw_lua();
            let loader = this.loader(rawlua.lua())?;
            rawlua.push(loader)?;
            Ok(1)
        })
    }

    (*config).is_require_allowed = is_require_allowed;
    (*config).reset = reset;
    (*config).jump_to_alias = jump_to_alias;
    (*config).to_alias_fallback = None;
    (*config).to_parent = to_parent;
    (*config).to_child = to_child;
    (*config).is_module_present = is_module_present;
    (*config).get_chunkname = get_chunkname;
    (*config).get_loadname = get_loadname;
    (*config).get_cache_key = get_cache_key;
    (*config).get_config_status = get_config_status;
    (*config).get_alias = None;
    (*config).get_config = Some(get_config);
    (*config).load = load;
}

/// Detect configuration file format (JSON or Luau)
#[cfg(feature = "luau")]
fn detect_config_format(data: &[u8]) -> ConfigStatus {
    let data = data.trim_ascii();
    if data.starts_with(b"{") {
        let data = &data[1..].trim_ascii_start();
        if data.starts_with(b"\"") || data == b"}" {
            return ConfigStatus::PresentJson;
        }
    }
    ConfigStatus::PresentLuau
}

/// Helper function to write data to a buffer
#[cfg(feature = "luau")]
unsafe fn write_to_buffer(
    buffer: *mut c_char,
    buffer_size: usize,
    size_out: *mut usize,
    data: &[u8],
) -> WriteResult {
    // the buffer must be null terminated as it's a c++ `std::string` data() buffer
    let is_null_terminated = data.last() == Some(&0);
    *size_out = data.len() + if is_null_terminated { 0 } else { 1 };
    if *size_out > buffer_size {
        return WriteResult::BufferTooSmall;
    }
    ptr::copy_nonoverlapping(data.as_ptr(), buffer as *mut _, data.len());
    if !is_null_terminated {
        *buffer.add(data.len()) = 0;
    }
    WriteResult::Success
}

#[cfg(feature = "luau")]
pub(super) fn create_require_function<R: Require + MaybeSend + 'static>(
    lua: &Lua,
    require: R,
) -> Result<Function> {
    unsafe extern "C-unwind" fn find_current_file(state: *mut ffi::lua_State) -> c_int {
        let mut ar: ffi::lua_Debug = mem::zeroed();
        for level in 2.. {
            if ffi::lua_getinfo(state, level, cstr!("s"), &mut ar) == 0 {
                ffi::luaL_error(state, cstr!("require is not supported in this context"));
            }
            if CStr::from_ptr(ar.what) != c"C" {
                break;
            }
        }
        ffi::lua_pushstring(state, ar.source);
        1
    }

    unsafe extern "C-unwind" fn get_cache_key(state: *mut ffi::lua_State) -> c_int {
        let ctx = ffi::lua_touserdata(state, ffi::lua_upvalueindex(1));
        let ctx = try_borrow!(state, ctx);
        let cache_key = ctx.cache_key();
        ffi::lua_pushlstring(state, cache_key.as_ptr() as *const _, cache_key.len());
        1
    }

    let (get_cache_key, find_current_file, proxyrequire, registered_modules, loader_cache) = unsafe {
        lua.exec_raw::<(Function, Function, Function, Table, Table)>((), move |state| {
            let context = Context::new(require);
            let context_ptr = ffi::lua_newuserdata_t(state, RefCell::new(context));
            ffi::lua_pushcclosured(state, get_cache_key, cstr!("get_cache_key"), 1);
            ffi::lua_pushcfunctiond(state, find_current_file, cstr!("find_current_file"));
            ffi::luarequire_pushproxyrequire(state, init_config, context_ptr as *mut _);
            ffi::luaL_getsubtable(state, ffi::LUA_REGISTRYINDEX, ffi::LUA_REGISTERED_MODULES_TABLE);
            ffi::luaL_getsubtable(state, ffi::LUA_REGISTRYINDEX, cstr!("__MLUA_LOADER_CACHE"));
        })
    }?;

    unsafe extern "C-unwind" fn error(state: *mut ffi::lua_State) -> c_int {
        ffi::luaL_where(state, 1);
        ffi::lua_pushvalue(state, 1);
        ffi::lua_concat(state, 2);
        ffi::lua_error(state);
    }

    unsafe extern "C-unwind" fn r#type(state: *mut ffi::lua_State) -> c_int {
        ffi::lua_pushstring(state, ffi::lua_typename(state, ffi::lua_type(state, 1)));
        1
    }

    unsafe extern "C-unwind" fn to_lowercase(state: *mut ffi::lua_State) -> c_int {
        let s = ffi::luaL_checkstring(state, 1);
        let s = CStr::from_ptr(s);
        if !s.to_bytes().iter().any(|&c| c.is_ascii_uppercase()) {
            // If the string does not contain any uppercase ASCII letters, return it as is
            return 1;
        }
        callback_error_ext(state, ptr::null_mut(), true, |extra, _| {
            let s = (s.to_bytes().iter())
                .map(|&c| c.to_ascii_lowercase())
                .collect::<bstr::BString>();
            (*extra).raw_lua().push(s).map(|_| 1)
        })
    }

    let (error, r#type, to_lowercase) = unsafe {
        lua.exec_raw::<(Function, Function, Function)>((), move |state| {
            ffi::lua_pushcfunctiond(state, error, cstr!("error"));
            ffi::lua_pushcfunctiond(state, r#type, cstr!("type"));
            ffi::lua_pushcfunctiond(state, to_lowercase, cstr!("to_lowercase"));
        })
    }?;

    // Prepare environment for the "require" function
    let env = lua.create_table_with_capacity(0, 7)?;
    env.raw_set("get_cache_key", get_cache_key)?;
    env.raw_set("find_current_file", find_current_file)?;
    env.raw_set("proxyrequire", proxyrequire)?;
    env.raw_set("REGISTERED_MODULES", registered_modules)?;
    env.raw_set("LOADER_CACHE", loader_cache)?;
    env.raw_set("error", error)?;
    env.raw_set("type", r#type)?;
    env.raw_set("to_lowercase", to_lowercase)?;

    lua.load(
        r#"
        local path = ...
        if type(path) ~= "string" then
            error("bad argument #1 to 'require' (string expected, got " .. type(path) .. ")")
        end

        -- Check if the module (path) is explicitly registered
        local maybe_result = REGISTERED_MODULES[to_lowercase(path)]
        if maybe_result ~= nil then
            return maybe_result
        end

        local loader = proxyrequire(path, find_current_file())
        local cache_key = get_cache_key()
        -- Check if the loader result is already cached
        local result = LOADER_CACHE[cache_key]
        if result ~= nil then
            return result
        end

        -- Call the loader function and cache the result
        result = loader()
        if result == nil then
            result = true
        end
        LOADER_CACHE[cache_key] = result
        return result
        "#,
    )
    .try_cache()
    .set_name("=__mlua_require")
    .set_environment(env)
    .into_function()
}

mod fs;
