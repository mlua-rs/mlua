use std::env;
use std::io;
use std::path::PathBuf;
use std::process::Command;

trait CommandExt {
    fn execute(&mut self) -> io::Result<()>;
}

impl CommandExt for Command {
    /// Execute the command and return an error if it exited with a failure status.
    fn execute(&mut self) -> io::Result<()> {
        self.status().map(|_| ()).map_err(|_| {
            io::Error::new(
                io::ErrorKind::Other,
                format!(
                    "The command\n\
                     \t{:?}\n\
                     did not run successfully.",
                    self
                ),
            )
        })
    }
}

fn main() {
    let lua = pkg_config::Config::new()
        .atleast_version("5.1")
        .probe("lua")
        .unwrap();

    let build_dir = PathBuf::from(env::var_os("OUT_DIR").unwrap());

    // Ensure the presence of glue.rs
    if !build_dir.join("glue.rs").exists() {
        let mut config = cc::Build::new();

        for path in lua.include_paths {
            config.include(path);
        }
        for (k, v) in lua.defines {
            config.define(&k, v.as_ref().map(|x| x.as_str()));
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
}
