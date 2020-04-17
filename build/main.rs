#![allow(unreachable_code)]

use std::env;
use std::io::{Error, ErrorKind, Result};
use std::path::{Path, PathBuf};
use std::process::Command;

#[cfg_attr(feature = "vendored", path = "find_vendored.rs")]
#[cfg_attr(not(feature = "vendored"), path = "find_normal.rs")]
mod find;

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

fn build_glue<P: AsRef<Path> + std::fmt::Debug>(include_path: &P) {
    let build_dir = PathBuf::from(env::var_os("OUT_DIR").unwrap());

    let mut config = cc::Build::new();
    config.include(include_path);

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
    #[cfg(not(any(
        feature = "lua53",
        feature = "lua52",
        feature = "lua51",
        feature = "luajit"
    )))]
    panic!("You must enable one of the features: lua53, lua52, lua51, luajit");

    #[cfg(all(
        feature = "lua53",
        any(feature = "lua52", feature = "lua51", feature = "luajit")
    ))]
    panic!("You can enable only one of the features: lua53, lua52, lua51, luajit");

    #[cfg(all(feature = "lua52", any(feature = "lua51", feature = "luajit")))]
    panic!("You can enable only one of the features: lua53, lua52, lua51, luajit");

    #[cfg(all(feature = "lua51", feature = "luajit"))]
    panic!("You can enable only one of the features: lua53, lua52, lua51, luajit");

    #[cfg(all(feature = "lua51", feature = "luajit"))]
    panic!("You can enable only one of the features: lua53, lua52, lua51, luajit");

    // Async
    // #[cfg(all(feature = "async", not(any(feature = "lua53", feature = "lua52"))))]
    // panic!("You can enable async only for: lua53, lua52");

    let include_dir = find::probe_lua();
    build_glue(&include_dir);
}
