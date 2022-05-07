#[cfg_attr(
    any(
        feature = "luau",
        all(
            feature = "vendored",
            any(
                feature = "lua54",
                feature = "lua53",
                feature = "lua52",
                feature = "lua51",
                feature = "luajit"
            )
        )
    ),
    path = "find_vendored.rs"
)]
#[cfg_attr(
    all(
        not(feature = "vendored"),
        any(
            feature = "lua54",
            feature = "lua53",
            feature = "lua52",
            feature = "lua51",
            feature = "luajit"
        )
    ),
    path = "find_normal.rs"
)]
#[cfg_attr(
    not(any(
        feature = "lua54",
        feature = "lua53",
        feature = "lua52",
        feature = "lua51",
        feature = "luajit",
        feature = "luau"
    )),
    path = "find_dummy.rs"
)]
mod find;

fn main() {
    #[cfg(not(any(
        feature = "lua54",
        feature = "lua53",
        feature = "lua52",
        feature = "lua51",
        feature = "luajit",
        feature = "luau"
    )))]
    compile_error!(
        "You must enable one of the features: lua54, lua53, lua52, lua51, luajit, luajit52, luau"
    );

    #[cfg(all(
        feature = "lua54",
        any(
            feature = "lua53",
            feature = "lua52",
            feature = "lua51",
            feature = "luajit",
            feature = "luau"
        )
    ))]
    compile_error!(
        "You can enable only one of the features: lua54, lua53, lua52, lua51, luajit, luajit52, luau"
    );

    #[cfg(all(
        feature = "lua53",
        any(
            feature = "lua52",
            feature = "lua51",
            feature = "luajit",
            feature = "luau"
        )
    ))]
    compile_error!(
        "You can enable only one of the features: lua54, lua53, lua52, lua51, luajit, luajit52, luau"
    );

    #[cfg(all(
        feature = "lua52",
        any(feature = "lua51", feature = "luajit", feature = "luau")
    ))]
    compile_error!(
        "You can enable only one of the features: lua54, lua53, lua52, lua51, luajit, luajit52, luau"
    );

    #[cfg(all(feature = "lua51", any(feature = "luajit", feature = "luau")))]
    compile_error!(
        "You can enable only one of the features: lua54, lua53, lua52, lua51, luajit, luajit52, luau"
    );

    #[cfg(all(feature = "luajit", feature = "luau"))]
    compile_error!(
        "You can enable only one of the features: lua54, lua53, lua52, lua51, luajit, luajit52, luau"
    );

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
