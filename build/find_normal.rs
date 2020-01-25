use std::env;
use std::ffi::OsString;
use std::fs::File;
use std::io::{BufRead, BufReader, Result};
use std::ops::Bound;
use std::path::{Path, PathBuf};

pub fn probe_lua() -> PathBuf {
    let include_dir = env::var_os("LUA_INC").unwrap_or(OsString::new());
    let lib_dir = env::var_os("LUA_LIB").unwrap_or(OsString::new());
    let lua_lib = env::var_os("LUA_LIB_NAME").unwrap_or(OsString::new());

    println!("cargo:rerun-if-env-changed=LUA_INC");
    println!("cargo:rerun-if-env-changed=LUA_LIB");
    println!("cargo:rerun-if-env-changed=LUA_LIB_NAME");
    println!("cargo:rerun-if-env-changed=LUA_LINK");

    if include_dir != "" && lib_dir != "" && lua_lib != "" {
        let _version = use_custom_lua(&include_dir, &lib_dir, &lua_lib).unwrap();
        return PathBuf::from(include_dir);
    }

    // Find using via pkg-config

    #[cfg(feature = "lua53")]
    {
        let mut lua = pkg_config::Config::new()
            .range_version((Bound::Included("5.3"), Bound::Excluded("5.4")))
            .probe("lua");

        if lua.is_err() {
            lua = pkg_config::Config::new().probe("lua5.3");
        }

        return lua.unwrap().include_paths[0].clone();
    }

    #[cfg(feature = "lua52")]
    {
        let mut lua = pkg_config::Config::new()
            .range_version((Bound::Included("5.2"), Bound::Excluded("5.3")))
            .probe("lua");

        if lua.is_err() {
            lua = pkg_config::Config::new().probe("lua5.2");
        }

        return lua.unwrap().include_paths[0].clone();
    }

    #[cfg(feature = "lua51")]
    {
        let mut lua = pkg_config::Config::new()
            .range_version((Bound::Included("5.1"), Bound::Excluded("5.2")))
            .probe("lua");

        if lua.is_err() {
            lua = pkg_config::Config::new().probe("lua5.1");
        }

        return lua.unwrap().include_paths[0].clone();
    }

    #[cfg(feature = "luajit")]
    {
        let lua = pkg_config::Config::new()
            .range_version((Bound::Included("2.1.0"), Bound::Unbounded))
            .probe("luajit");

        return lua.unwrap().include_paths[0].clone();
    }
}

fn use_custom_lua<S: AsRef<Path>>(include_dir: &S, lib_dir: &S, lua_lib: &S) -> Result<String> {
    let mut version_found = String::new();

    // Find LUA_VERSION_NUM
    let mut lua_h_path = include_dir.as_ref().to_owned();
    lua_h_path.push("lua.h");
    let f = File::open(lua_h_path)?;
    let reader = BufReader::new(f);
    for line in reader.lines() {
        let line = line?;
        let parts = line.split_whitespace().collect::<Vec<_>>();
        if parts.len() == 3 && parts[1] == "LUA_VERSION_NUM" {
            version_found = parts[2].to_string();
        }
    }

    let mut link_lib = String::new();
    if env::var("LUA_LINK").unwrap_or(String::new()) == "static" {
        link_lib = "static=".to_string();
    }

    println!(
        "cargo:rustc-link-search=native={}",
        lib_dir.as_ref().display()
    );
    println!(
        "cargo:rustc-link-lib={}{}",
        link_lib,
        lua_lib.as_ref().display()
    );

    Ok(version_found)
}
