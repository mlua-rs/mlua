#![allow(dead_code)]

use std::env;
use std::ops::Bound;

pub fn probe_lua() {
    let target_arch = env::var("CARGO_CFG_TARGET_ARCH").unwrap();

    if target_arch == "wasm32" && cfg!(not(feature = "vendored")) {
        panic!("Please enable `vendored` feature to build for wasm32");
    }

    let lib_dir = env::var("LUA_LIB").unwrap_or_default();
    let lua_lib = env::var("LUA_LIB_NAME").unwrap_or_default();

    println!("cargo:rerun-if-env-changed=LUA_LIB");
    println!("cargo:rerun-if-env-changed=LUA_LIB_NAME");
    println!("cargo:rerun-if-env-changed=LUA_LINK");

    if !lua_lib.is_empty() {
        if !lib_dir.is_empty() {
            println!("cargo:rustc-link-search=native={lib_dir}");
        }
        let mut link_lib = "";
        if env::var("LUA_LINK").as_deref() == Ok("static") {
            link_lib = "static=";
        };
        println!("cargo:rustc-link-lib={link_lib}{lua_lib}");
        return;
    }

    // Find using `pkg-config`

    #[cfg(feature = "lua54")]
    let (incl_bound, excl_bound, alt_probe, ver) =
        ("5.4", "5.5", ["lua5.4", "lua-5.4", "lua54"], "5.4");
    #[cfg(feature = "lua53")]
    let (incl_bound, excl_bound, alt_probe, ver) =
        ("5.3", "5.4", ["lua5.3", "lua-5.3", "lua53"], "5.3");
    #[cfg(feature = "lua52")]
    let (incl_bound, excl_bound, alt_probe, ver) =
        ("5.2", "5.3", ["lua5.2", "lua-5.2", "lua52"], "5.2");
    #[cfg(feature = "lua51")]
    let (incl_bound, excl_bound, alt_probe, ver) =
        ("5.1", "5.2", ["lua5.1", "lua-5.1", "lua51"], "5.1");
    #[cfg(feature = "luajit")]
    let (incl_bound, excl_bound, alt_probe, ver) = ("2.0.4", "2.2", [], "JIT");

    #[rustfmt::skip]
    let mut lua = pkg_config::Config::new()
        .range_version((Bound::Included(incl_bound), Bound::Excluded(excl_bound)))
        .cargo_metadata(true)
        .probe(if cfg!(feature = "luajit") { "luajit" } else { "lua" });

    if lua.is_err() {
        for pkg in alt_probe {
            lua = pkg_config::Config::new()
                .cargo_metadata(true)
                .probe(pkg);

            if lua.is_ok() {
                break;
            }
        }
    }

    lua.unwrap_or_else(|err| panic!("cannot find Lua{ver} using `pkg-config`: {err}"));
}
