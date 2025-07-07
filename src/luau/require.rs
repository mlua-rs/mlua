use std::cell::RefCell;
use std::collections::VecDeque;
use std::ffi::CStr;
use std::io::Result as IoResult;
use std::ops::{Deref, DerefMut};
use std::os::raw::{c_char, c_int, c_void};
use std::path::{Component, Path, PathBuf};
use std::result::Result as StdResult;
use std::{env, fmt, fs, mem, ptr};

use crate::error::{Error, Result};
use crate::function::Function;
use crate::state::{callback_error_ext, Lua};
use crate::table::Table;
use crate::types::MaybeSend;

/// An error that can occur during navigation in the Luau `require-by-string` system.
#[cfg(any(feature = "luau", doc))]
#[cfg_attr(docsrs, doc(cfg(feature = "luau")))]
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

/// A trait for handling modules loading and navigation in the Luau `require-by-string` system.
#[cfg(any(feature = "luau", doc))]
#[cfg_attr(docsrs, doc(cfg(feature = "luau")))]
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

    /// Returns whether the context is currently pointing at a module
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

/// The standard implementation of Luau `require-by-string` navigation.
#[derive(Default, Debug)]
pub struct TextRequirer {
    /// An absolute path to the current Luau module (not mapped to a physical file)
    abs_path: PathBuf,
    /// A relative path to the current Luau module (not mapped to a physical file)
    rel_path: PathBuf,
    /// A physical path to the current Luau module, which is a file or a directory with an
    /// `init.lua(u)` file
    resolved_path: Option<PathBuf>,
}

impl TextRequirer {
    /// The prefix used for chunk names in the require system.
    /// Only chunk names starting with this prefix are allowed to be used in `require`.
    const CHUNK_PREFIX: &str = "@";

    /// The file extensions that are considered valid for Luau modules.
    const FILE_EXTENSIONS: &[&str] = &["luau", "lua"];

    /// Creates a new `TextRequirer` instance.
    pub fn new() -> Self {
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

    /// Resolve a Luau module path to a physical file or directory.
    ///
    /// Empty directories without init files are considered valid as "intermediate" directories.
    fn resolve_module(path: &Path) -> StdResult<Option<PathBuf>, NavigateError> {
        let mut found_path = None;

        if path.components().next_back() != Some(Component::Normal("init".as_ref())) {
            let current_ext = (path.extension().and_then(|s| s.to_str()))
                .map(|s| format!("{s}."))
                .unwrap_or_default();
            for ext in Self::FILE_EXTENSIONS {
                let candidate = path.with_extension(format!("{current_ext}{ext}"));
                if candidate.is_file() && found_path.replace(candidate).is_some() {
                    return Err(NavigateError::Ambiguous);
                }
            }
        }
        if path.is_dir() {
            for component in Self::FILE_EXTENSIONS.iter().map(|ext| format!("init.{ext}")) {
                let candidate = path.join(component);
                if candidate.is_file() && found_path.replace(candidate).is_some() {
                    return Err(NavigateError::Ambiguous);
                }
            }

            if found_path.is_none() {
                // Directories without init files are considered valid "intermediate" path
                return Ok(None);
            }
        }

        Ok(Some(found_path.ok_or(NavigateError::NotFound)?))
    }
}

impl Require for TextRequirer {
    fn is_require_allowed(&self, chunk_name: &str) -> bool {
        chunk_name.starts_with(Self::CHUNK_PREFIX)
    }

    fn reset(&mut self, chunk_name: &str) -> StdResult<(), NavigateError> {
        if !chunk_name.starts_with(Self::CHUNK_PREFIX) {
            return Err(NavigateError::NotFound);
        }
        let chunk_name = Self::normalize_chunk_name(&chunk_name[1..]);
        let chunk_path = Self::normalize_path(chunk_name.as_ref());

        if chunk_path.extension() == Some("rs".as_ref()) {
            // Special case for Rust source files, reset to the current directory
            let chunk_filename = chunk_path.file_name().unwrap();
            let cwd = env::current_dir().map_err(|_| NavigateError::NotFound)?;
            self.abs_path = Self::normalize_path(&cwd.join(chunk_filename));
            self.rel_path = ([Component::CurDir, Component::Normal(chunk_filename)].into_iter()).collect();
            self.resolved_path = None;

            return Ok(());
        }

        if chunk_path.is_absolute() {
            let resolved_path = Self::resolve_module(&chunk_path)?;
            self.abs_path = chunk_path.clone();
            self.rel_path = chunk_path;
            self.resolved_path = resolved_path;
        } else {
            // Relative path
            let cwd = env::current_dir().map_err(|_| NavigateError::NotFound)?;
            let abs_path = Self::normalize_path(&cwd.join(&chunk_path));
            let resolved_path = Self::resolve_module(&abs_path)?;
            self.abs_path = abs_path;
            self.rel_path = chunk_path;
            self.resolved_path = resolved_path;
        }

        Ok(())
    }

    fn jump_to_alias(&mut self, path: &str) -> StdResult<(), NavigateError> {
        let path = Self::normalize_path(path.as_ref());
        let resolved_path = Self::resolve_module(&path)?;

        self.abs_path = path.clone();
        self.rel_path = path;
        self.resolved_path = resolved_path;

        Ok(())
    }

    fn to_parent(&mut self) -> StdResult<(), NavigateError> {
        let mut abs_path = self.abs_path.clone();
        if !abs_path.pop() {
            // It's important to return `NotFound` if we reached the root, as it's a "recoverable" error if we
            // cannot go beyond the root directory.
            // Luau "require-by-string` has a special logic to search for config file to resolve aliases.
            return Err(NavigateError::NotFound);
        }
        let mut rel_parent = self.rel_path.clone();
        rel_parent.pop();
        let resolved_path = Self::resolve_module(&abs_path)?;

        self.abs_path = abs_path;
        self.rel_path = Self::normalize_path(&rel_parent);
        self.resolved_path = resolved_path;

        Ok(())
    }

    fn to_child(&mut self, name: &str) -> StdResult<(), NavigateError> {
        let abs_path = self.abs_path.join(name);
        let rel_path = self.rel_path.join(name);
        let resolved_path = Self::resolve_module(&abs_path)?;

        self.abs_path = abs_path;
        self.rel_path = rel_path;
        self.resolved_path = resolved_path;

        Ok(())
    }

    fn has_module(&self) -> bool {
        (self.resolved_path.as_deref())
            .map(Path::is_file)
            .unwrap_or(false)
    }

    fn cache_key(&self) -> String {
        self.resolved_path.as_deref().unwrap().display().to_string()
    }

    fn has_config(&self) -> bool {
        self.abs_path.is_dir() && self.abs_path.join(".luaurc").is_file()
    }

    fn config(&self) -> IoResult<Vec<u8>> {
        fs::read(self.abs_path.join(".luaurc"))
    }

    fn loader(&self, lua: &Lua) -> Result<Function> {
        let name = format!("@{}", self.rel_path.display());
        lua.load(self.resolved_path.as_deref().unwrap())
            .set_name(name)
            .into_function()
    }
}

struct Context(Box<dyn Require>);

impl Deref for Context {
    type Target = dyn Require;

    fn deref(&self) -> &Self::Target {
        &*self.0
    }
}

impl DerefMut for Context {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut *self.0
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

    unsafe extern "C-unwind" fn is_config_present(state: *mut ffi::lua_State, ctx: *mut c_void) -> bool {
        let this = try_borrow!(state, ctx);
        this.has_config()
    }

    unsafe extern "C-unwind" fn get_config(
        state: *mut ffi::lua_State,
        ctx: *mut c_void,
        buffer: *mut c_char,
        buffer_size: usize,
        size_out: *mut usize,
    ) -> WriteResult {
        let this = try_borrow!(state, ctx);
        let config = callback_error_ext(state, ptr::null_mut(), true, move |_, _| Ok(this.config()?));
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
    (*config).to_parent = to_parent;
    (*config).to_child = to_child;
    (*config).is_module_present = is_module_present;
    (*config).get_chunkname = get_chunkname;
    (*config).get_loadname = get_loadname;
    (*config).get_cache_key = get_cache_key;
    (*config).is_config_present = is_config_present;
    (*config).get_alias = None;
    (*config).get_config = Some(get_config);
    (*config).load = load;
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
            let context = Context(Box::new(require));
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
