use std::env;
use std::path::PathBuf;

use mlua::{Lua, Result};

#[test]
fn test_module_simple() -> Result<()> {
    let lua = make_lua()?;
    lua.load(
        r#"
        local mod = require("test_module")
        assert(mod.sum(2,2) == 4)
    "#,
    )
    .exec()
}

#[test]
fn test_module_multi() -> Result<()> {
    let lua = make_lua()?;
    lua.load(
        r#"
        local mod = require("test_module")
        local mod2 = require("test_module.second")
        assert(mod.check_userdata(mod2.userdata) == 123)
    "#,
    )
    .exec()
}

#[test]
fn test_module_error() -> Result<()> {
    let lua = make_lua()?;
    lua.load(
        r#"
        local ok, err = pcall(require, "test_module.error")
        assert(not ok)
        assert(string.find(tostring(err), "custom module error"))
    "#,
    )
    .exec()
}

#[cfg(any(
    feature = "lua54",
    feature = "lua53",
    feature = "lua52",
    feature = "lua51"
))]
#[test]
fn test_module_from_thread() -> Result<()> {
    let lua = make_lua()?;
    lua.load(
        r#"
        local mod

        local co = coroutine.create(function(a, b)
            mod = require("test_module")
            assert(mod.sum(a, b) == a + b)
        end)

        local ok, err = coroutine.resume(co, 3, 5)
        assert(ok, err)
        collectgarbage()

        assert(mod.used_memory() > 0)
    "#,
    )
    .exec()
}

#[cfg(any(
    feature = "lua54",
    feature = "lua53",
    feature = "lua52",
    feature = "lua51"
))]
#[test]
fn test_module_multi_from_thread() -> Result<()> {
    let lua = make_lua()?;
    lua.load(
        r#"
        local mod = require("test_module")
        local co = coroutine.create(function()
            local mod2 = require("test_module.second")
            assert(mod2.userdata ~= nil)
        end)
        local ok, err = coroutine.resume(co)
        assert(ok, err)
    "#,
    )
    .exec()
}

#[test]
fn test_module_new_vm() -> Result<()> {
    let lua = make_lua()?;
    lua.load(
        r#"
        local mod = require("test_module.new_vm")
        assert(mod.eval("return \"hello, world\"") == "hello, world")
    "#,
    )
    .exec()
}

fn make_lua() -> Result<Lua> {
    let (dylib_path, dylib_ext, separator);
    if cfg!(target_os = "macos") {
        dylib_path = env::var("DYLD_FALLBACK_LIBRARY_PATH").unwrap();
        dylib_ext = "dylib";
        separator = ":";
    } else if cfg!(target_os = "linux") {
        dylib_path = env::var("LD_LIBRARY_PATH").unwrap();
        dylib_ext = "so";
        separator = ":";
    } else if cfg!(target_os = "windows") {
        dylib_path = env::var("PATH").unwrap();
        dylib_ext = "dll";
        separator = ";";
    } else {
        panic!("unknown target os");
    };

    let mut cpath = dylib_path
        .split(separator)
        .take(3)
        .map(|p| {
            let mut path = PathBuf::from(p);
            path.push(format!("lib?.{}", dylib_ext));
            path.to_str().unwrap().to_owned()
        })
        .collect::<Vec<_>>()
        .join(";");

    if cfg!(target_os = "windows") {
        cpath = cpath.replace("\\", "\\\\");
        cpath = cpath.replace("lib?.", "?.");
    }

    let lua = unsafe { Lua::unsafe_new() }; // To be able to load C modules
    lua.load(&format!("package.cpath = \"{}\"", cpath)).exec()?;
    Ok(lua)
}
