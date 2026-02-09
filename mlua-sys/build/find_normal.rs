#![allow(dead_code)]

use std::env;

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

    // This reads [package.metadata.system-deps] from Cargo.toml
    system_deps::Config::new().probe().unwrap();
}
