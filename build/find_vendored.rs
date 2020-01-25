use std::path::PathBuf;

#[cfg(feature = "lua-vendored")]
use lua_src;
#[cfg(feature = "luajit-vendored")]
use luajit_src;

pub fn probe_lua() -> PathBuf {
    #[cfg(all(feature = "lua53", feature = "lua-vendored"))]
    let artifacts = lua_src::Build::new().build(lua_src::Lua53);
    #[cfg(all(feature = "lua52", feature = "lua-vendored"))]
    let artifacts = lua_src::Build::new().build(lua_src::Lua52);
    #[cfg(all(feature = "lua51", feature = "lua-vendored"))]
    let artifacts = lua_src::Build::new().build(lua_src::Lua51);
    #[cfg(feature = "luajit-vendored")]
    let artifacts = luajit_src::Build::new().build();

    #[cfg(all(feature = "luajit", feature = "lua-vendored"))]
    let artifacts = lua_src::Build::new().build(lua_src::Lua51); // Invalid case! Workaround to get panic

    artifacts.print_cargo_metadata();
    artifacts.include_dir().to_owned()
}
