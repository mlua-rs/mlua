use std::ffi::CStr;
use std::fmt::Write;
use std::os::raw::{c_float, c_int};
use std::path::{PathBuf, MAIN_SEPARATOR_STR};
use std::string::String as StdString;
use std::{env, fs};

use crate::chunk::ChunkMode;
use crate::error::Result;
use crate::lua::Lua;
use crate::table::Table;
use crate::types::RegistryKey;
use crate::value::{IntoLua, Value};

// Since Luau has some missing standard function, we re-implement them here

// We keep reference to the `package` table in registry under this key
struct PackageKey(RegistryKey);

impl Lua {
    pub(crate) unsafe fn prepare_luau_state(&self) -> Result<()> {
        let globals = self.globals();

        globals.raw_set(
            "collectgarbage",
            self.create_c_function(lua_collectgarbage)?,
        )?;
        globals.raw_set("require", self.create_c_function(lua_require)?)?;
        globals.raw_set("package", create_package_table(self)?)?;
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

//
// package module
//

fn create_package_table(lua: &Lua) -> Result<Table> {
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

    // Set `package.loaded` (table with a list of loaded modules)
    let loaded = lua.create_table()?;
    package.raw_set("loaded", loaded.clone())?;
    lua.set_named_registry_value("_LOADED", loaded)?;

    // Set `package.loaders`
    let loaders = lua.create_sequence_from([lua.create_function(lua_loader)?])?;
    package.raw_set("loaders", loaders.clone())?;
    lua.set_named_registry_value("_LOADERS", loaders)?;

    Ok(package)
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
