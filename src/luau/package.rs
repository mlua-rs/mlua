use std::ffi::CStr;
use std::fmt::Write;
use std::os::raw::c_int;
use std::path::{PathBuf, MAIN_SEPARATOR_STR};
use std::string::String as StdString;
use std::{env, fs};

use crate::chunk::ChunkMode;
use crate::error::Result;
use crate::lua::Lua;
use crate::table::Table;
use crate::types::RegistryKey;
use crate::value::{IntoLua, Value};

#[cfg(unix)]
use {libloading::Library, rustc_hash::FxHashMap};

//
// Luau package module
//

#[cfg(unix)]
const TARGET_MLUA_LUAU_ABI_VERSION: u32 = 1;

#[cfg(all(unix, feature = "module"))]
#[no_mangle]
#[used]
pub static MLUA_LUAU_ABI_VERSION: u32 = TARGET_MLUA_LUAU_ABI_VERSION;

// We keep reference to the `package` table in registry under this key
struct PackageKey(RegistryKey);

// We keep reference to the loaded dylibs in application data
#[cfg(unix)]
struct LoadedDylibs(FxHashMap<PathBuf, Library>);

#[cfg(unix)]
impl std::ops::Deref for LoadedDylibs {
    type Target = FxHashMap<PathBuf, Library>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[cfg(unix)]
impl std::ops::DerefMut for LoadedDylibs {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

pub(crate) fn register_package_module(lua: &Lua) -> Result<()> {
    // Create the package table and store it in app_data for later use (bypassing globals lookup)
    let package = lua.create_table()?;
    lua.set_app_data(PackageKey(lua.create_registry_value(package.clone())?));

    // Set `package.path`
    let mut search_path = env::var("LUAU_PATH")
        .or_else(|_| env::var("LUA_PATH"))
        .unwrap_or_default();
    if search_path.is_empty() {
        search_path = "?.luau;?.lua".to_string();
    }
    package.raw_set("path", search_path)?;

    // Set `package.cpath`
    #[cfg(unix)]
    {
        let mut search_cpath = env::var("LUAU_CPATH")
            .or_else(|_| env::var("LUA_CPATH"))
            .unwrap_or_default();
        if search_cpath.is_empty() {
            if cfg!(any(target_os = "macos", target_os = "ios")) {
                search_cpath = "?.dylib".to_string();
            } else {
                search_cpath = "?.so".to_string();
            }
        }
        package.raw_set("cpath", search_cpath)?;
    }

    // Set `package.loaded` (table with a list of loaded modules)
    let loaded = lua.create_table()?;
    package.raw_set("loaded", loaded.clone())?;
    lua.set_named_registry_value("_LOADED", loaded)?;

    // Set `package.loaders`
    let loaders = lua.create_sequence_from([lua.create_function(lua_loader)?])?;
    package.raw_set("loaders", loaders.clone())?;
    #[cfg(unix)]
    {
        loaders.push(lua.create_function(dylib_loader)?)?;
        lua.set_app_data(LoadedDylibs(FxHashMap::default()));
    }
    lua.set_named_registry_value("_LOADERS", loaders)?;

    // Register the module and `require` function in globals
    let globals = lua.globals();
    globals.raw_set("package", package)?;
    globals.raw_set("require", unsafe { lua.create_c_function(lua_require)? })?;

    Ok(())
}

#[allow(unused_variables)]
pub(crate) fn disable_dylibs(lua: &Lua) {
    // Presence of `LoadedDylibs` in app data is used as a flag
    // to check whether binary modules are enabled
    #[cfg(unix)]
    lua.remove_app_data::<LoadedDylibs>();
}

unsafe extern "C-unwind" fn lua_require(state: *mut ffi::lua_State) -> c_int {
    ffi::lua_settop(state, 1);
    let name = ffi::luaL_checkstring(state, 1);
    ffi::luaL_getsubtable(state, ffi::LUA_REGISTRYINDEX, cstr!("_LOADED")); // _LOADED is at index 2
    if ffi::lua_rawgetfield(state, 2, name) != ffi::LUA_TNIL {
        return 1; // module is already loaded
    }
    ffi::lua_pop(state, 1); // remove nil

    // load the module
    let err_buf = ffi::lua_newuserdata_t::<StdString>(state);
    err_buf.write(StdString::new());
    ffi::luaL_getsubtable(state, ffi::LUA_REGISTRYINDEX, cstr!("_LOADERS")); // _LOADERS is at index 3
    for i in 1.. {
        if ffi::lua_rawgeti(state, -1, i) == ffi::LUA_TNIL {
            // no more loaders?
            if (*err_buf).is_empty() {
                ffi::luaL_error(state, cstr!("module '%s' not found"), name);
            } else {
                let bytes = (*err_buf).as_bytes();
                let extra = ffi::lua_pushlstring(state, bytes.as_ptr() as *const _, bytes.len());
                ffi::luaL_error(state, cstr!("module '%s' not found:%s"), name, extra);
            }
        }
        ffi::lua_pushvalue(state, 1); // name arg
        ffi::lua_call(state, 1, 2); // call loader
        match ffi::lua_type(state, -2) {
            ffi::LUA_TFUNCTION => break, // loader found
            ffi::LUA_TSTRING => {
                // error message
                let msg = ffi::lua_tostring(state, -2);
                let msg = CStr::from_ptr(msg).to_string_lossy();
                _ = write!(&mut *err_buf, "\n\t{msg}");
            }
            _ => {}
        }
        ffi::lua_pop(state, 2); // remove both results
    }
    ffi::lua_pushvalue(state, 1); // name is 1st argument to module loader
    ffi::lua_rotate(state, -2, 1); // loader data <-> name

    // stack: ...; loader function; module name; loader data
    ffi::lua_call(state, 2, 1);
    // stack: ...; result from loader function
    if ffi::lua_isnil(state, -1) != 0 {
        ffi::lua_pop(state, 1);
        ffi::lua_pushboolean(state, 1); // use true as result
    }
    ffi::lua_pushvalue(state, -1); // make copy of entrypoint result
    ffi::lua_setfield(state, 2, name); /* _LOADED[name] = returned value */
    1
}

/// Searches for the given `name` in the given `path`.
///
/// `path` is a string containing a sequence of templates separated by semicolons.
fn package_searchpath(name: &str, search_path: &str, try_prefix: bool) -> Option<PathBuf> {
    let mut names = vec![name.replace('.', MAIN_SEPARATOR_STR)];
    if try_prefix && name.contains('.') {
        let prefix = name.split_once('.').map(|(prefix, _)| prefix).unwrap();
        names.push(prefix.to_string());
    }
    for path in search_path.split(';') {
        for name in &names {
            let file_path = PathBuf::from(path.replace('?', name));
            if let Ok(true) = fs::metadata(&file_path).map(|m| m.is_file()) {
                return Some(file_path);
            }
        }
    }
    None
}

//
// Module loaders
//

/// Tries to load a lua (text) file
fn lua_loader(lua: &Lua, modname: StdString) -> Result<Value> {
    let package = {
        let key = lua.app_data_ref::<PackageKey>().unwrap();
        lua.registry_value::<Table>(&key.0)
    }?;
    let search_path = package.get::<_, StdString>("path").unwrap_or_default();

    if let Some(file_path) = package_searchpath(&modname, &search_path, false) {
        match fs::read(&file_path) {
            Ok(buf) => {
                return lua
                    .load(&buf)
                    .set_name(&format!("={}", file_path.display()))
                    .set_mode(ChunkMode::Text)
                    .into_function()
                    .map(Value::Function);
            }
            Err(err) => {
                return format!("cannot open '{}': {err}", file_path.display()).into_lua(lua);
            }
        }
    }

    Ok(Value::Nil)
}

/// Tries to load a dynamic library
#[cfg(unix)]
fn dylib_loader(lua: &Lua, modname: StdString) -> Result<Value> {
    let package = {
        let key = lua.app_data_ref::<PackageKey>().unwrap();
        lua.registry_value::<Table>(&key.0)
    }?;
    let search_cpath = package.get::<_, StdString>("cpath").unwrap_or_default();

    let find_symbol = |lib: &Library| unsafe {
        if let Ok(entry) = lib.get::<ffi::lua_CFunction>(format!("luaopen_{modname}\0").as_bytes())
        {
            return lua.create_c_function(*entry).map(Value::Function);
        }
        // Try all in one mode
        if let Ok(entry) = lib.get::<ffi::lua_CFunction>(
            format!("luaopen_{}\0", modname.replace('.', "_")).as_bytes(),
        ) {
            return lua.create_c_function(*entry).map(Value::Function);
        }
        "cannot find module entrypoint".into_lua(lua)
    };

    if let Some(file_path) = package_searchpath(&modname, &search_cpath, true) {
        let file_path = file_path.canonicalize()?;
        // Load the library and check for symbol
        unsafe {
            let mut loaded_dylibs = match lua.app_data_mut::<LoadedDylibs>() {
                Some(loaded_dylibs) => loaded_dylibs,
                None => return "dynamic libraries are disabled in safe mode".into_lua(lua),
            };
            // Check if it's already loaded
            if let Some(lib) = loaded_dylibs.get(&file_path) {
                return find_symbol(lib);
            }
            if let Ok(lib) = Library::new(&file_path) {
                // Check version
                let mod_version = lib.get::<*const u32>(b"MLUA_LUAU_ABI_VERSION");
                let mod_version = mod_version.map(|v| **v).unwrap_or_default();
                if mod_version != TARGET_MLUA_LUAU_ABI_VERSION {
                    let err = format!("wrong module ABI version (expected {TARGET_MLUA_LUAU_ABI_VERSION}, got {mod_version})");
                    return err.into_lua(lua);
                }
                let symbol = find_symbol(&lib);
                loaded_dylibs.insert(file_path, lib);
                return symbol;
            }
        }
    }

    Ok(Value::Nil)
}
