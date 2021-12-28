use std::env;
use std::ops::Bound;
use std::path::PathBuf;

fn get_env_var(name: &str) -> String {
    match env::var(name) {
        Ok(val) => val,
        Err(env::VarError::NotPresent) => String::new(),
        Err(err) => panic!("cannot get {}: {}", name, err),
    }
}

pub fn probe_lua() -> Option<PathBuf> {
    let include_dir = get_env_var("LUA_INC");
    let lib_dir = get_env_var("LUA_LIB");
    let lua_lib = get_env_var("LUA_LIB_NAME");

    println!("cargo:rerun-if-env-changed=LUA_INC");
    println!("cargo:rerun-if-env-changed=LUA_LIB");
    println!("cargo:rerun-if-env-changed=LUA_LIB_NAME");
    println!("cargo:rerun-if-env-changed=LUA_LINK");

    let need_lua_lib = cfg!(any(not(feature = "module"), target_os = "windows"));

    if !include_dir.is_empty() {
        if need_lua_lib {
            if lib_dir.is_empty() {
                panic!("LUA_LIB is not set");
            }
            if lua_lib.is_empty() {
                panic!("LUA_LIB_NAME is not set");
            }

            let mut link_lib = "";
            if get_env_var("LUA_LINK") == "static" {
                link_lib = "static=";
            };
            println!("cargo:rustc-link-search=native={}", lib_dir);
            println!("cargo:rustc-link-lib={}{}", link_lib, lua_lib);
        }
        return Some(PathBuf::from(include_dir));
    }

    // Find using `pkg-config`

    #[cfg(feature = "lua54")]
    let (incl_bound, excl_bound, alt_probe, ver) = ("5.4", "5.5", "lua5.4", "5.4");
    #[cfg(feature = "lua53")]
    let (incl_bound, excl_bound, alt_probe, ver) = ("5.3", "5.4", "lua5.3", "5.3");
    #[cfg(feature = "lua52")]
    let (incl_bound, excl_bound, alt_probe, ver) = ("5.2", "5.3", "lua5.2", "5.2");
    #[cfg(feature = "lua51")]
    let (incl_bound, excl_bound, alt_probe, ver) = ("5.1", "5.2", "lua5.1", "5.1");

    #[cfg(any(
        feature = "lua54",
        feature = "lua53",
        feature = "lua52",
        feature = "lua51"
    ))]
    {
        let mut lua = pkg_config::Config::new()
            .range_version((Bound::Included(incl_bound), Bound::Excluded(excl_bound)))
            .cargo_metadata(need_lua_lib)
            .probe("lua");

        if lua.is_err() {
            lua = pkg_config::Config::new()
                .cargo_metadata(need_lua_lib)
                .probe(alt_probe);
        }

        lua.expect(&format!("cannot find Lua {} using `pkg-config`", ver))
            .include_paths
            .get(0)
            .cloned()
    }

    #[cfg(feature = "luajit")]
    {
        let lua = pkg_config::Config::new()
            .range_version((Bound::Included("2.0.5"), Bound::Unbounded))
            .cargo_metadata(need_lua_lib)
            .probe("luajit");

        lua.expect("cannot find LuaJIT using `pkg-config`")
            .include_paths
            .get(0)
            .cloned()
    }
}
