use std::env;
use std::fs::File;
use std::io::{BufRead, BufReader, Error, ErrorKind, Result};
use std::ops::Bound;
use std::path::{Path, PathBuf};
use std::process::Command;

trait CommandExt {
    fn execute(&mut self) -> Result<()>;
}

impl CommandExt for Command {
    /// Execute the command and return an error if it exited with a failure status.
    fn execute(&mut self) -> Result<()> {
        self.status()
            .and_then(|status| {
                if status.success() {
                    Ok(())
                } else {
                    Err(Error::new(ErrorKind::Other, "non-zero exit code"))
                }
            })
            .map_err(|_| {
                Error::new(
                    ErrorKind::Other,
                    format!("The command {:?} did not run successfully.", self),
                )
            })
    }
}

fn use_custom_lua<S: AsRef<str>>(include_dir: &S, lib_dir: &S, lua_lib: &S) -> Result<String> {
    let mut version_found = String::new();

    // Find LUA_VERSION_NUM
    let mut lua_h_path = PathBuf::from(include_dir.as_ref());
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

    let mut static_link = "";
    if env::var("LUA_LINK").unwrap_or(String::new()) == "static" {
        static_link = "static=";
    }

    println!("cargo:rustc-link-search=native={}", lib_dir.as_ref());
    println!("cargo:rustc-link-lib={}{}", static_link, lua_lib.as_ref());

    Ok(version_found)
}

fn build_glue<P: AsRef<Path> + std::fmt::Debug>(include_paths: &[P]) {
    let build_dir = PathBuf::from(env::var_os("OUT_DIR").unwrap());

    // Ensure the presence of glue.rs
    // if build_dir.join("glue.rs").exists() {
    //     return;
    // }

    let mut config = cc::Build::new();

    for path in include_paths {
        config.include(path);
    }

    // Compile and run glue.c
    let glue = build_dir.join("glue");

    config
        .get_compiler()
        .to_command()
        .arg("src/ffi/glue/glue.c")
        .arg("-o")
        .arg(&glue)
        .execute()
        .unwrap();

    Command::new(glue)
        .arg(build_dir.join("glue.rs"))
        .execute()
        .unwrap();
}

fn main() {
    let include_dir = env::var("LUA_INC").unwrap_or(String::new());
    let lib_dir = env::var("LUA_LIB").unwrap_or(String::new());
    let lua_lib = env::var("LUA_LIB_NAME").unwrap_or(String::new());

    println!("cargo:rerun-if-env-changed=LUA_INC");
    println!("cargo:rerun-if-env-changed=LUA_LIB");
    println!("cargo:rerun-if-env-changed=LUA_LIB_NAME");
    println!("cargo:rerun-if-env-changed=LUA_LINK");
    println!("cargo:rerun-if-changed=src/ffi/glue/glue.c");

    if include_dir != "" && lib_dir != "" && lua_lib != "" {
        let _version = use_custom_lua(&include_dir, &lib_dir, &lua_lib).unwrap();
        build_glue(&[include_dir]);
        return;
    }

    // Find lua via pkg-config

    #[cfg(feature = "lua53")]
    {
        let mut lua = pkg_config::Config::new()
            .range_version((Bound::Included("5.3"), Bound::Excluded("5.4")))
            .probe("lua");

        if lua.is_err() {
            lua = pkg_config::Config::new().probe("lua5.3");
        }

        match lua {
            Ok(lua) => build_glue(&lua.include_paths),
            Err(err) => panic!(err),
        };
    }

    #[cfg(not(feature = "lua53"))]
    {
        let mut lua = pkg_config::Config::new()
            .range_version((Bound::Included("5.1"), Bound::Excluded("5.2")))
            .probe("lua");

        if lua.is_err() {
            lua = pkg_config::Config::new().probe("lua5.1");
        }

        match lua {
            Ok(lua) => build_glue(&lua.include_paths),
            Err(err) => panic!(err),
        };
    }
}
