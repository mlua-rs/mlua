use std::path::PathBuf;

#[cfg(any(feature = "lua53", feature = "lua52", feature = "lua51"))]
use lua_src;
#[cfg(feature = "luajit")]
use luajit_src;

pub fn probe_lua() -> PathBuf {
    #[cfg(feature = "lua53")]
    let artifacts = lua_src::Build::new().build(lua_src::Lua53);
    #[cfg(feature = "lua52")]
    let artifacts = lua_src::Build::new().build(lua_src::Lua52);
    #[cfg(feature = "lua51")]
    let artifacts = lua_src::Build::new().build(lua_src::Lua51);
    #[cfg(feature = "luajit")]
    let artifacts = luajit_src::Build::new().build();

    artifacts.print_cargo_metadata();
    artifacts.include_dir().to_owned()
}
