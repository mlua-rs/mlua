#![allow(dead_code)]

pub fn probe_lua() {
    #[cfg(feature = "lua54")]
    let artifacts = lua_src::Build::new().build(lua_src::Lua54);

    #[cfg(feature = "lua53")]
    let artifacts = lua_src::Build::new().build(lua_src::Lua53);

    #[cfg(feature = "lua52")]
    let artifacts = lua_src::Build::new().build(lua_src::Lua52);

    #[cfg(feature = "lua51")]
    let artifacts = lua_src::Build::new().build(lua_src::Lua51);

    #[cfg(feature = "luajit")]
    let artifacts = luajit_src::Build::new()
        .lua52compat(cfg!(feature = "luajit52"))
        .build();

    #[cfg(feature = "luau")]
    let artifacts = luau0_src::Build::new()
        .enable_codegen(cfg!(feature = "luau-codegen"))
        .set_max_cstack_size(1000000)
        .set_vector_size(if cfg!(feature = "luau-vector4") { 4 } else { 3 })
        .build();

    artifacts.print_cargo_metadata();
}
