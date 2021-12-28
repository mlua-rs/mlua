use std::path::PathBuf;

pub fn probe_lua() -> Option<PathBuf> {
    #[cfg(feature = "lua54")]
    let artifacts = lua_src::Build::new().build(lua_src::Lua54);
    #[cfg(feature = "lua53")]
    let artifacts = lua_src::Build::new().build(lua_src::Lua53);
    #[cfg(feature = "lua52")]
    let artifacts = lua_src::Build::new().build(lua_src::Lua52);
    #[cfg(feature = "lua51")]
    let artifacts = lua_src::Build::new().build(lua_src::Lua51);
    #[cfg(feature = "luajit")]
    let artifacts = {
        let mut builder = luajit_src::Build::new();
        if cfg!(feature = "luajit52") {
            builder.lua52compat(true);
        }
        builder.build()
    };

    #[cfg(not(feature = "module"))]
    artifacts.print_cargo_metadata();

    Some(artifacts.include_dir().to_owned())
}
