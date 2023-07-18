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
    #[cfg(all(feature = "luau", feature = "module"))]
    compile_error!("Luau does not support module mode");

    // "vendored, module" mode makes sense (only) on windows
    #[cfg(any(
        not(feature = "module"),
        all(not(feature = "vendored"), target_os = "windows")
    ))]
    find::probe_lua();

    println!("cargo:rerun-if-changed=build");
}
