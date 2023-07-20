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
    compile_error!("Luau does not support `module` mode");

    #[cfg(all(feature = "module", feature = "vendored"))]
    compile_error!("`vendored` and `module` features are mutually exclusive");

    #[cfg(not(feature = "module"))]
    find::probe_lua();

    println!("cargo:rerun-if-changed=build");
}
