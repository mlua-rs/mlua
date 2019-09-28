#[allow(non_camel_case_types)]
type lua_State = libc::c_void;

#[allow(non_camel_case_types)]
type lua_CFunction = unsafe extern "C" fn(L: *mut lua_State) -> libc::c_int;

extern "C" {
    fn luaL_newstate() -> *mut lua_State;

    fn luaL_requiref(
        L: *mut lua_State,
        modname: *const libc::c_char,
        openf: lua_CFunction,
        glb: libc::c_int,
    );

    fn lua_settop(L: *mut lua_State, idx: libc::c_int);

    fn luaopen_base(L: *mut lua_State) -> libc::c_int;
    fn luaopen_coroutine(L: *mut lua_State) -> libc::c_int;
    fn luaopen_table(L: *mut lua_State) -> libc::c_int;
    fn luaopen_io(L: *mut lua_State) -> libc::c_int;
    fn luaopen_os(L: *mut lua_State) -> libc::c_int;
    fn luaopen_string(L: *mut lua_State) -> libc::c_int;
    fn luaopen_math(L: *mut lua_State) -> libc::c_int;
    fn luaopen_package(L: *mut lua_State) -> libc::c_int;
}

#[allow(unused)]
fn make_lua() -> rlua::Lua {
    macro_rules! cstr {
        ($s:expr) => {
            concat!($s, "\0") as *const str as *const ::std::os::raw::c_char
        };
    }

    unsafe {
        let state = luaL_newstate();

        // Do not open the debug library, it can be used to cause unsafety.
        luaL_requiref(state, cstr!("_G"), luaopen_base, 1);
        luaL_requiref(state, cstr!("coroutine"), luaopen_coroutine, 1);
        luaL_requiref(state, cstr!("table"), luaopen_table, 1);
        luaL_requiref(state, cstr!("io"), luaopen_io, 1);
        luaL_requiref(state, cstr!("os"), luaopen_os, 1);
        luaL_requiref(state, cstr!("string"), luaopen_string, 1);
        luaL_requiref(state, cstr!("math"), luaopen_math, 1);
        luaL_requiref(state, cstr!("package"), luaopen_package, 1);
        lua_settop(state, -8 - 1);

        rlua::Lua::init_from_ptr(state)
    }
}
