use std::env;

cfg_if::cfg_if! {
    if #[cfg(any(feature = "luau", feature = "vendored"))] {
        #[path = "find_vendored.rs"]
        mod find;
    } else {
        #[path = "find_normal.rs"]
        mod find;
    }
}

fn main() {
    #[cfg(all(feature = "luau", feature = "module", windows))]
    compile_error!("Luau does not support `module` mode on Windows");

    #[cfg(any(
        all(feature = "vendored", any(feature = "external", feature = "module")),
        all(feature = "external", any(feature = "vendored", feature = "module")),
        all(feature = "module", any(feature = "vendored", feature = "external"))
    ))]
    compile_error!("`vendored`, `external` and `module` features are mutually exclusive");

    println!("cargo:rerun-if-changed=build");

    // Check if compilation and linking is handled by external crate
    if cfg!(not(feature = "external")) {
        let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap();
        if target_os == "windows" && cfg!(feature = "module") {
            if !std::env::var("LUA_LIB_NAME").unwrap_or_default().is_empty() {
                // Don't use raw-dylib linking
                find::probe_lua();
                return;
            }

            println!("cargo:rustc-cfg=raw_dylib");
        }

        #[cfg(not(feature = "module"))]
        find::probe_lua();
    }
}
