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
    // We don't support "vendored module" mode on windows
    #[cfg(all(feature = "vendored", feature = "module", target_os = "windows"))]
    compile_error!(
        "Vendored (static) builds are not supported for modules on Windows.\n"
            + "Please, use `pkg-config` or custom mode to link to a Lua dll."
    );

    #[cfg(all(feature = "luau", feature = "module"))]
    compile_error!("Luau does not support module mode");

    #[cfg(any(not(feature = "module"), target_os = "windows"))]
    find::probe_lua();

    println!("cargo:rerun-if-changed=build");
}
