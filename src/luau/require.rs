use std::cell::RefCell;
use std::collections::VecDeque;
use std::ffi::CStr;
use std::io::Result as IoResult;
use std::os::raw::{c_char, c_int, c_void};
use std::path::{Component, Path, PathBuf};
use std::result::Result as StdResult;
use std::{env, fmt, fs, mem, ptr};
use crate::error::{Result, Error};
use crate::function::Function;
use crate::state::{callback_error_ext, Lua};
use crate::table::Table;
use crate::types::MaybeSend;
use crate::traits::IntoLua;
use crate::state::RawLua;

/// An error that can occur during navigation in the Luau `require` system.
pub enum NavigateError {
    Ambiguous,
    NotFound,
    Error(Error)
}

#[cfg(feature = "luau")]
trait IntoNavigateResult {
    fn into_nav_result(self) -> ffi::luarequire_NavigateResult;
}

#[cfg(feature = "luau")]
impl IntoNavigateResult for StdResult<(), NavigateError> {
    fn into_nav_result(self) -> ffi::luarequire_NavigateResult {
        match self {
            Ok(()) => ffi::luarequire_NavigateResult::Success,
            Err(NavigateError::Ambiguous) => ffi::luarequire_NavigateResult::Ambiguous,
            Err(NavigateError::NotFound) => ffi::luarequire_NavigateResult::NotFound,
            Err(NavigateError::Error(_)) => unreachable!()
        }
    }
}

#[cfg(feature = "luau")]
type WriteResult = ffi::luarequire_WriteResult;

/// A trait for handling modules loading and navigation in the Luau `require` system.
pub trait Require: MaybeSend {
    /// Returns `true` if "require" is permitted for the given chunk name.
    fn is_require_allowed(&self, chunk_name: &str) -> bool;

    /// Resets the internal state to point at the requirer module.
    fn reset(&self, chunk_name: &str) -> StdResult<(), NavigateError>;

    /// Resets the internal state to point at an aliased module.
    ///
    /// This function received an exact path from a configuration file.
    /// It's only called when an alias's path cannot be resolved relative to its
    /// configuration file.
    fn jump_to_alias(&self, path: &str) -> StdResult<(), NavigateError>;

    // Navigate to parent directory
    fn to_parent(&self) -> StdResult<(), NavigateError>;

    /// Navigate to the given child directory.
    fn to_child(&self, name: &str) -> StdResult<(), NavigateError>;

    /// Returns whether the context is currently pointing at a module
    fn is_module_present(&self) -> bool;

    /// Returns the contents of the current module
    ///
    /// This function is only called if `is_module_present` returns true.
    fn contents(&self) -> IoResult<Vec<u8>>;

    /// Returns a chunk name for the current module.
    ///
    /// This function is only called if `is_module_present` returns true.
    /// The chunk name is used to identify the module using the debug library.
    fn chunk_name(&self) -> String;

    /// Provides a cache key representing the current module.
    ///
    /// This function is only called if `is_module_present` returns true.
    fn cache_key(&self) -> Vec<u8>;

    /// Returns whether a configuration file is present in the current context.
    fn is_config_present(&self) -> bool;

    /// Returns the contents of the configuration file in the current context.
    ///
    /// This function is only called if `is_config_present` returns true.
    fn config(&self) -> IoResult<Vec<u8>>;

    /// Returns a loader that when called, loads the module and returns the result.
    ///
    /// Loader can be sync or async.
    fn loader(&self, lua: &Lua, path: &str, chunk_name: &str, content: &[u8]) -> Result<Function> {
        let _ = path;
        lua.load(content).set_name(chunk_name).into_function()
    }
}

impl fmt::Debug for dyn Require {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "<dyn Require>")
    }
}

/// The standard implementation of Luau `require` navigation.
#[derive(Default)]
pub(super) struct TextRequirer {
    abs_path: RefCell<PathBuf>,
    rel_path: RefCell<PathBuf>,
    module_path: RefCell<PathBuf>,
}

impl TextRequirer {
    pub(super) fn new() -> Self {
        Self::default()
    }

    fn normalize_chunk_name(chunk_name: &str) -> &str {
        if let Some((path, line)) = chunk_name.split_once(':') {
            if line.parse::<u32>().is_ok() {
                return path;
            }
        }
        chunk_name
    }

    // Normalizes the path by removing unnecessary components
    fn normalize_path(path: &Path) -> PathBuf {
        let mut components = VecDeque::new();

        for comp in path.components() {
            match comp {
                Component::Prefix(..) | Component::RootDir => {
                    components.push_back(comp);
                }
                Component::CurDir => {}
                Component::ParentDir => {
                    if matches!(components.back(), None | Some(Component::ParentDir)) {
                        components.push_back(Component::ParentDir);
                    } else if matches!(components.back(), Some(Component::Normal(..))) {
                        components.pop_back();
                    }
                }
                Component::Normal(..) => components.push_back(comp),
            }
        }

        if matches!(components.front(), None | Some(Component::Normal(..))) {
            components.push_front(Component::CurDir);
        }

        // Join the components back together
        components.into_iter().collect()
    }

    fn find_module_path(path: &Path) -> StdResult<PathBuf, NavigateError> {
        let mut found_path = None;

        let current_ext = (path.extension().and_then(|s| s.to_str()))
            .map(|s| format!("{s}."))
            .unwrap_or_default();
        for ext in ["luau", "lua"] {
            let candidate = path.with_extension(format!("{current_ext}{ext}"));
            if candidate.is_file() {
                if found_path.is_some() {
                    return Err(NavigateError::Ambiguous);
                }
                found_path = Some(candidate);
            }
        }
        if path.is_dir() {
            if found_path.is_some() {
                return Err(NavigateError::Ambiguous);
            }

            for component in ["init.luau", "init.lua"] {
                let candidate = path.join(component);
                if candidate.is_file() {
                    if found_path.is_some() {
                        return Err(NavigateError::Ambiguous);
                    }
                    found_path = Some(candidate);
                }
            }

            if found_path.is_none() {
                found_path = Some(PathBuf::new());
            }
        }

        found_path.ok_or(NavigateError::NotFound)
    }
}

impl Require for TextRequirer {
    fn is_require_allowed(&self, chunk_name: &str) -> bool {
        chunk_name.starts_with('@')
    }

    fn reset(&self, chunk_name: &str) -> StdResult<(), NavigateError> {
        if !chunk_name.starts_with('@') {
            return Err(NavigateError::NotFound);
        }
        let chunk_name = &Self::normalize_chunk_name(chunk_name)[1..];
        let path = Self::normalize_path(chunk_name.as_ref());

        if path.extension() == Some("rs".as_ref()) {
            let cwd = match env::current_dir() {
                Ok(cwd) => cwd,
                Err(_) => return Err(NavigateError::NotFound),
            };
            self.abs_path.replace(Self::normalize_path(&cwd.join(&path)));
            self.rel_path.replace(path);
            self.module_path.replace(PathBuf::new());

            return Ok(());
        }

        if path.is_absolute() {
            let module_path = Self::find_module_path(&path)?;
            self.abs_path.replace(path.clone());
            self.rel_path.replace(path);
            self.module_path.replace(module_path);
        } else {
            // Relative path
            let cwd = match env::current_dir() {
                Ok(cwd) => cwd,
                Err(_) => return Err(NavigateError::NotFound),
            };
            let abs_path = cwd.join(&path);
            let module_path = Self::find_module_path(&abs_path)?;
            self.abs_path.replace(Self::normalize_path(&abs_path));
            self.rel_path.replace(path);
            self.module_path.replace(module_path);
        }

        Ok(())
    }

    fn jump_to_alias(&self, path: &str) -> StdResult<(), NavigateError> {
        let path = Self::normalize_path(path.as_ref());
        let module_path = Self::find_module_path(&path)?;

        self.abs_path.replace(path.clone());
        self.rel_path.replace(path);
        self.module_path.replace(module_path);

        Ok(())
    }

    fn to_parent(&self) -> StdResult<(), NavigateError> {
        let mut abs_path = self.abs_path.borrow().clone();
        if !abs_path.pop() {
            return Err(NavigateError::NotFound);
        }
        let mut rel_parent = self.rel_path.borrow().clone();
        rel_parent.pop();
        let module_path = Self::find_module_path(&abs_path)?;

        self.abs_path.replace(abs_path);
        self.rel_path.replace(Self::normalize_path(&rel_parent));
        self.module_path.replace(module_path);

        Ok(())
    }

    fn to_child(&self, name: &str) -> StdResult<(), NavigateError> {
        let abs_path = self.abs_path.borrow().join(name);
        let rel_path = self.rel_path.borrow().join(name);
        let module_path = Self::find_module_path(&abs_path)?;

        self.abs_path.replace(abs_path);
        self.rel_path.replace(rel_path);
        self.module_path.replace(module_path);

        Ok(())
    }

    fn is_module_present(&self) -> bool {
        self.module_path.borrow().is_file()
    }

    fn contents(&self) -> IoResult<Vec<u8>> {
        fs::read(&*self.module_path.borrow())
    }

    fn chunk_name(&self) -> String {
        format!("@{}", self.rel_path.borrow().display())
    }

    fn cache_key(&self) -> Vec<u8> {
        self.module_path.borrow().display().to_string().into_bytes()
    }

    fn is_config_present(&self) -> bool {
        self.abs_path.borrow().join(".luaurc").is_file()
    }

    fn config(&self) -> IoResult<Vec<u8>> {
        fs::read(self.abs_path.borrow().join(".luaurc"))
    }
}

#[cfg(feature = "luau")]
pub(super) unsafe extern "C" fn init_config(config: *mut ffi::luarequire_Configuration) {
    if config.is_null() {
        return;
    }

    unsafe extern "C" fn is_require_allowed(
        _state: *mut ffi::lua_State,
        ctx: *mut c_void,
        requirer_chunkname: *const c_char,
    ) -> bool {
        if requirer_chunkname.is_null() {
            return false;
        }

        let this = &*(ctx as *const Box<dyn Require>);
        let chunk_name = CStr::from_ptr(requirer_chunkname).to_string_lossy();
        this.is_require_allowed(&chunk_name)
    }

    unsafe extern "C-unwind" fn reset(
        state: *mut ffi::lua_State,
        ctx: *mut c_void,
        requirer_chunkname: *const c_char,
    ) -> ffi::luarequire_NavigateResult {
        let this = &*(ctx as *const Box<dyn Require>);
        let chunk_name = CStr::from_ptr(requirer_chunkname).to_string_lossy();
        match this.reset(&chunk_name) {
            Err(NavigateError::Error(err)) => {
                let raw_lua = RawLua::init_from_ptr(state, false);
                err.push_into_stack(&raw_lua.lock()).expect("mlua internal: failed to push error to stack");
                ffi::lua_error(state);
            },
            error => error.into_nav_result()		
        }
    }

    unsafe extern "C-unwind" fn jump_to_alias(
        state: *mut ffi::lua_State,
        ctx: *mut c_void,
        path: *const c_char,
    ) -> ffi::luarequire_NavigateResult {
        let this = &*(ctx as *const Box<dyn Require>);
        let path = CStr::from_ptr(path).to_string_lossy();
        match this.jump_to_alias(&path) {
            Err(NavigateError::Error(err)) => {
                let raw_lua = RawLua::init_from_ptr(state, false);
                err.push_into_stack(&raw_lua.lock()).expect("mlua internal: failed to push error to stack");
                ffi::lua_error(state);
            },
            error => error.into_nav_result()
        }
    }

    unsafe extern "C-unwind" fn to_parent(
        state: *mut ffi::lua_State,
        ctx: *mut c_void,
    ) -> ffi::luarequire_NavigateResult {
        let this = &*(ctx as *const Box<dyn Require>);
        match this.to_parent() {
            Err(NavigateError::Error(err)) => {
                let raw_lua = RawLua::init_from_ptr(state, false);
                err.push_into_stack(&raw_lua.lock()).expect("mlua internal: failed to push error to stack");
                ffi::lua_error(state);
            },
            error => error.into_nav_result()
        }
    }

    unsafe extern "C-unwind" fn to_child(
        state: *mut ffi::lua_State,
        ctx: *mut c_void,
        name: *const c_char,
    ) -> ffi::luarequire_NavigateResult {
        let this = &*(ctx as *const Box<dyn Require>);
        let name = CStr::from_ptr(name).to_string_lossy();
        match this.to_child(&name) {
            Err(NavigateError::Error(err)) => {
                let raw_lua = RawLua::init_from_ptr(state, false);
                err.push_into_stack(&raw_lua.lock()).expect("mlua internal: failed to push error to stack");
                ffi::lua_error(state);
            },
            error => error.into_nav_result()
        }
    }

    unsafe extern "C" fn is_module_present(_state: *mut ffi::lua_State, ctx: *mut c_void) -> bool {
        let this = &*(ctx as *const Box<dyn Require>);
        this.is_module_present()
    }

    unsafe extern "C" fn get_contents(
        state: *mut ffi::lua_State,
        ctx: *mut c_void,
        buffer: *mut c_char,
        buffer_size: usize,
        size_out: *mut usize,
    ) -> WriteResult {
        let this = &*(ctx as *const Box<dyn Require>);
        write_to_buffer(state, buffer, buffer_size, size_out, || this.contents())
    }

    unsafe extern "C" fn get_chunkname(
        state: *mut ffi::lua_State,
        ctx: *mut c_void,
        buffer: *mut c_char,
        buffer_size: usize,
        size_out: *mut usize,
    ) -> WriteResult {
        let this = &*(ctx as *const Box<dyn Require>);
        write_to_buffer(state, buffer, buffer_size, size_out, || {
            Ok(this.chunk_name().into_bytes())
        })
    }

    unsafe extern "C" fn get_cache_key(
        state: *mut ffi::lua_State,
        ctx: *mut c_void,
        buffer: *mut c_char,
        buffer_size: usize,
        size_out: *mut usize,
    ) -> WriteResult {
        let this = &*(ctx as *const Box<dyn Require>);
        write_to_buffer(state, buffer, buffer_size, size_out, || Ok(this.cache_key()))
    }

    unsafe extern "C" fn is_config_present(_state: *mut ffi::lua_State, ctx: *mut c_void) -> bool {
        let this = &*(ctx as *const Box<dyn Require>);
        this.is_config_present()
    }

    unsafe extern "C" fn get_config(
        state: *mut ffi::lua_State,
        ctx: *mut c_void,
        buffer: *mut c_char,
        buffer_size: usize,
        size_out: *mut usize,
    ) -> WriteResult {
        let this = &*(ctx as *const Box<dyn Require>);
        write_to_buffer(state, buffer, buffer_size, size_out, || this.config())
    }

    unsafe extern "C-unwind" fn load(
        state: *mut ffi::lua_State,
        ctx: *mut c_void,
        path: *const c_char,
        chunk_name: *const c_char,
        contents: *const c_char,
    ) -> c_int {
        let this = &*(ctx as *const Box<dyn Require>);
        let path = CStr::from_ptr(path).to_string_lossy();
        let chunk_name = CStr::from_ptr(chunk_name).to_string_lossy();
        let contents = CStr::from_ptr(contents).to_bytes();
        callback_error_ext(state, ptr::null_mut(), false, move |extra, _| {
            let rawlua = (*extra).raw_lua();
            rawlua.push(this.loader(rawlua.lua(), &path, &chunk_name, contents)?)?;
            Ok(1)
        })
    }

    (*config).is_require_allowed = is_require_allowed;
    (*config).reset = reset;
    (*config).jump_to_alias = jump_to_alias;
    (*config).to_parent = to_parent;
    (*config).to_child = to_child;
    (*config).is_module_present = is_module_present;
    (*config).get_contents = get_contents;
    (*config).get_chunkname = get_chunkname;
    (*config).get_cache_key = get_cache_key;
    (*config).is_config_present = is_config_present;
    (*config).get_config = get_config;
    (*config).load = load;
}

/// Helper function to write data to a buffer
#[cfg(feature = "luau")]
unsafe fn write_to_buffer(
    state: *mut ffi::lua_State,
    buffer: *mut c_char,
    buffer_size: usize,
    size_out: *mut usize,
    data_fetcher: impl Fn() -> IoResult<Vec<u8>>,
) -> WriteResult {
    struct DataCache(Option<Vec<u8>>);

    // The initial buffer size can be too small, to avoid making a second data fetch call,
    // we cache the content in the first call, and then re-use it.

    let lua = Lua::get_or_init_from_ptr(state);
    match lua.try_app_data_mut::<DataCache>() {
        Ok(Some(mut data_cache)) => {
            if let Some(data) = data_cache.0.take() {
                mlua_assert!(data.len() <= buffer_size, "buffer is too small");
                *size_out = data.len();
                ptr::copy_nonoverlapping(data.as_ptr(), buffer as *mut _, data.len());
                return WriteResult::Success;
            }
        }
        Ok(None) => {
            // Init the cache
            _ = lua.try_set_app_data(DataCache(None));
        }
        Err(_) => {}
    }

    match data_fetcher() {
        Ok(data) => {
            *size_out = data.len();
            if *size_out > buffer_size {
                // Cache the data for the next call to avoid getting the contents again
                if let Ok(Some(mut data_cache)) = lua.try_app_data_mut::<DataCache>() {
                    data_cache.0 = Some(data);
                }
                return WriteResult::BufferTooSmall;
            }
            ptr::copy_nonoverlapping(data.as_ptr(), buffer as *mut _, data.len());
            WriteResult::Success
        }
        Err(_) => WriteResult::Failure,
    }
}

#[cfg(feature = "luau")]
pub fn create_require_function<R: Require + 'static>(lua: &Lua, require: R) -> Result<Function> {
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
        let requirer = ffi::lua_touserdata(state, ffi::lua_upvalueindex(1)) as *const Box<dyn Require>;
        let cache_key = (*requirer).cache_key();
        ffi::lua_pushlstring(state, cache_key.as_ptr() as *const _, cache_key.len());
        1
    }

    let (get_cache_key, find_current_file, proxyrequire, registered_modules, loader_cache) = unsafe {
        lua.exec_raw::<(Function, Function, Function, Table, Table)>((), move |state| {
            let requirer_ptr = ffi::lua_newuserdata_t::<Box<dyn Require>>(state, Box::new(require));
            ffi::lua_pushcclosured(state, get_cache_key, cstr!("get_cache_key"), 1);
            ffi::lua_pushcfunctiond(state, find_current_file, cstr!("find_current_file"));
            ffi::luarequire_pushproxyrequire(state, init_config, requirer_ptr as *mut _);
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

    let (error, r#type) = unsafe {
        lua.exec_raw::<(Function, Function)>((), move |state| {
            ffi::lua_pushcfunctiond(state, error, cstr!("error"));
            ffi::lua_pushcfunctiond(state, r#type, cstr!("type"));
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

    lua.load(
        r#"
        local path = ...
        if type(path) ~= "string" then
            error("bad argument #1 to 'require' (string expected, got " .. type(path) .. ")")
        end

        -- Check if the module (path) is explicitly registered
        local maybe_result = REGISTERED_MODULES[path]
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

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::TextRequirer;

    #[test]
    fn test_path_normalize() {
        for (input, expected) in [
            // Basic formatting checks
            ("", "./"),
            (".", "./"),
            ("a/relative/path", "./a/relative/path"),
            // Paths containing extraneous '.' and '/' symbols
            ("./remove/extraneous/symbols/", "./remove/extraneous/symbols"),
            ("./remove/extraneous//symbols", "./remove/extraneous/symbols"),
            ("./remove/extraneous/symbols/.", "./remove/extraneous/symbols"),
            ("./remove/extraneous/./symbols", "./remove/extraneous/symbols"),
            ("../remove/extraneous/symbols/", "../remove/extraneous/symbols"),
            ("../remove/extraneous//symbols", "../remove/extraneous/symbols"),
            ("../remove/extraneous/symbols/.", "../remove/extraneous/symbols"),
            ("../remove/extraneous/./symbols", "../remove/extraneous/symbols"),
            ("/remove/extraneous/symbols/", "/remove/extraneous/symbols"),
            ("/remove/extraneous//symbols", "/remove/extraneous/symbols"),
            ("/remove/extraneous/symbols/.", "/remove/extraneous/symbols"),
            ("/remove/extraneous/./symbols", "/remove/extraneous/symbols"),
            // Paths containing '..'
            ("./remove/me/..", "./remove"),
            ("./remove/me/../", "./remove"),
            ("../remove/me/..", "../remove"),
            ("../remove/me/../", "../remove"),
            ("/remove/me/..", "/remove"),
            ("/remove/me/../", "/remove"),
            ("./..", "../"),
            ("./../", "../"),
            ("../..", "../../"),
            ("../../", "../../"),
            // '..' disappears if path is absolute and component is non-erasable
            ("/../", "/"),
        ] {
            let path = TextRequirer::normalize_path(input.as_ref());
            assert_eq!(
                &path,
                expected.as_ref() as &Path,
                "wrong normalization for {input}"
            );
        }
    }
}
