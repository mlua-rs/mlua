//! Contains definitions from `lua.h`.

use std::ffi::CStr;
use std::marker::{PhantomData, PhantomPinned};
use std::os::raw::{c_char, c_double, c_float, c_int, c_uint, c_void};
use std::{mem, ptr};

// Option for multiple returns in 'lua_pcall' and 'lua_call'
pub const LUA_MULTRET: c_int = -1;

// Max number of Lua stack slots
const LUAI_MAXCSTACK: c_int = 1000000;

// Number of valid Lua userdata tags
pub const LUA_UTAG_LIMIT: c_int = 128;

// Number of valid Lua lightuserdata tags
pub const LUA_LUTAG_LIMIT: c_int = 128;

//
// Pseudo-indices
//
pub const LUA_REGISTRYINDEX: c_int = -LUAI_MAXCSTACK - 2000;
pub const LUA_ENVIRONINDEX: c_int = -LUAI_MAXCSTACK - 2001;
pub const LUA_GLOBALSINDEX: c_int = -LUAI_MAXCSTACK - 2002;

pub const fn lua_upvalueindex(i: c_int) -> c_int {
    LUA_GLOBALSINDEX - i
}

//
// Thread status
//
pub const LUA_OK: c_int = 0;
pub const LUA_YIELD: c_int = 1;
pub const LUA_ERRRUN: c_int = 2;
pub const LUA_ERRSYNTAX: c_int = 3;
pub const LUA_ERRMEM: c_int = 4;
pub const LUA_ERRERR: c_int = 5;

/// A raw Lua state associated with a thread.
#[repr(C)]
pub struct lua_State {
    _data: [u8; 0],
    _marker: PhantomData<(*mut u8, PhantomPinned)>,
}

//
// Basic types
//
pub const LUA_TNONE: c_int = -1;

pub const LUA_TNIL: c_int = 0;
pub const LUA_TBOOLEAN: c_int = 1;

pub const LUA_TLIGHTUSERDATA: c_int = 2;
pub const LUA_TNUMBER: c_int = 3;
pub const LUA_TVECTOR: c_int = 4;

pub const LUA_TSTRING: c_int = 5;
pub const LUA_TTABLE: c_int = 6;
pub const LUA_TFUNCTION: c_int = 7;
pub const LUA_TUSERDATA: c_int = 8;
pub const LUA_TTHREAD: c_int = 9;
pub const LUA_TBUFFER: c_int = 10;

/// Guaranteed number of Lua stack slots available to a C function.
pub const LUA_MINSTACK: c_int = 20;

/// A Lua number, usually equivalent to `f64`.
pub type lua_Number = c_double;

/// A Lua integer, usually equivalent to `i64`
#[cfg(target_pointer_width = "32")]
pub type lua_Integer = i32;
#[cfg(target_pointer_width = "64")]
pub type lua_Integer = i64;

/// A Lua unsigned integer, equivalent to `u32`.
pub type lua_Unsigned = c_uint;

/// Type for native C functions that can be passed to Lua.
pub type lua_CFunction = unsafe extern "C-unwind" fn(L: *mut lua_State) -> c_int;
pub type lua_Continuation = unsafe extern "C-unwind" fn(L: *mut lua_State, status: c_int) -> c_int;

/// Type for userdata destructor functions (no unwinding).
pub type lua_Destructor = unsafe extern "C" fn(L: *mut lua_State, *mut c_void);

/// Type for memory-allocation functions (no unwinding).
pub type lua_Alloc =
    unsafe extern "C" fn(ud: *mut c_void, ptr: *mut c_void, osize: usize, nsize: usize) -> *mut c_void;

/// Returns Luau release version (eg. `0.xxx`).
pub const fn luau_version() -> Option<&'static str> {
    option_env!("LUAU_VERSION")
}

unsafe extern "C-unwind" {
    //
    // State manipulation
    //
    pub fn lua_newstate(f: lua_Alloc, ud: *mut c_void) -> *mut lua_State;
    pub fn lua_close(L: *mut lua_State);
    pub fn lua_newthread(L: *mut lua_State) -> *mut lua_State;
    pub fn lua_mainthread(L: *mut lua_State) -> *mut lua_State;
    pub fn lua_resetthread(L: *mut lua_State);
    pub fn lua_isthreadreset(L: *mut lua_State) -> c_int;

    //
    // Basic stack manipulation
    //
    pub fn lua_absindex(L: *mut lua_State, idx: c_int) -> c_int;
    pub fn lua_gettop(L: *mut lua_State) -> c_int;
    pub fn lua_settop(L: *mut lua_State, idx: c_int);
    pub fn lua_pushvalue(L: *mut lua_State, idx: c_int);
    pub fn lua_remove(L: *mut lua_State, idx: c_int);
    pub fn lua_insert(L: *mut lua_State, idx: c_int);
    pub fn lua_replace(L: *mut lua_State, idx: c_int);
    pub fn lua_checkstack(L: *mut lua_State, sz: c_int) -> c_int;
    pub fn lua_rawcheckstack(L: *mut lua_State, sz: c_int);

    pub fn lua_xmove(from: *mut lua_State, to: *mut lua_State, n: c_int);
    pub fn lua_xpush(from: *mut lua_State, to: *mut lua_State, idx: c_int);

    //
    // Access functions (stack -> C)
    //
    pub fn lua_isnumber(L: *mut lua_State, idx: c_int) -> c_int;
    pub fn lua_isstring(L: *mut lua_State, idx: c_int) -> c_int;
    pub fn lua_iscfunction(L: *mut lua_State, idx: c_int) -> c_int;
    pub fn lua_isLfunction(L: *mut lua_State, idx: c_int) -> c_int;
    pub fn lua_isuserdata(L: *mut lua_State, idx: c_int) -> c_int;
    pub fn lua_type(L: *mut lua_State, idx: c_int) -> c_int;
    pub fn lua_typename(L: *mut lua_State, tp: c_int) -> *const c_char;

    pub fn lua_equal(L: *mut lua_State, idx1: c_int, idx2: c_int) -> c_int;
    pub fn lua_rawequal(L: *mut lua_State, idx1: c_int, idx2: c_int) -> c_int;
    pub fn lua_lessthan(L: *mut lua_State, idx1: c_int, idx2: c_int) -> c_int;

    pub fn lua_tonumberx(L: *mut lua_State, idx: c_int, isnum: *mut c_int) -> lua_Number;
    #[link_name = "lua_tointegerx"]
    pub fn lua_tointegerx_(L: *mut lua_State, idx: c_int, isnum: *mut c_int) -> c_int;
    pub fn lua_tounsignedx(L: *mut lua_State, idx: c_int, isnum: *mut c_int) -> lua_Unsigned;
    pub fn lua_tovector(L: *mut lua_State, idx: c_int) -> *const c_float;
    pub fn lua_toboolean(L: *mut lua_State, idx: c_int) -> c_int;
    pub fn lua_tolstring(L: *mut lua_State, idx: c_int, len: *mut usize) -> *const c_char;
    pub fn lua_tostringatom(L: *mut lua_State, idx: c_int, atom: *mut c_int) -> *const c_char;
    pub fn lua_namecallatom(L: *mut lua_State, atom: *mut c_int) -> *const c_char;
    pub fn lua_objlen(L: *mut lua_State, idx: c_int) -> usize;
    pub fn lua_tocfunction(L: *mut lua_State, idx: c_int) -> Option<lua_CFunction>;
    pub fn lua_tolightuserdata(L: *mut lua_State, idx: c_int) -> *mut c_void;
    pub fn lua_tolightuserdatatagged(L: *mut lua_State, idx: c_int, tag: c_int) -> *mut c_void;
    pub fn lua_touserdata(L: *mut lua_State, idx: c_int) -> *mut c_void;
    pub fn lua_touserdatatagged(L: *mut lua_State, idx: c_int, tag: c_int) -> *mut c_void;
    pub fn lua_userdatatag(L: *mut lua_State, idx: c_int) -> c_int;
    pub fn lua_lightuserdatatag(L: *mut lua_State, idx: c_int) -> c_int;
    pub fn lua_tothread(L: *mut lua_State, idx: c_int) -> *mut lua_State;
    pub fn lua_tobuffer(L: *mut lua_State, idx: c_int, len: *mut usize) -> *mut c_void;
    pub fn lua_topointer(L: *mut lua_State, idx: c_int) -> *const c_void;

    //
    // Push functions (C -> stack)
    //
    pub fn lua_pushnil(L: *mut lua_State);
    pub fn lua_pushnumber(L: *mut lua_State, n: lua_Number);
    #[link_name = "lua_pushinteger"]
    pub fn lua_pushinteger_(L: *mut lua_State, n: c_int);
    pub fn lua_pushunsigned(L: *mut lua_State, n: lua_Unsigned);
    #[cfg(not(feature = "luau-vector4"))]
    pub fn lua_pushvector(L: *mut lua_State, x: c_float, y: c_float, z: c_float);
    #[cfg(feature = "luau-vector4")]
    pub fn lua_pushvector(L: *mut lua_State, x: c_float, y: c_float, z: c_float, w: c_float);
    #[link_name = "lua_pushlstring"]
    pub fn lua_pushlstring_(L: *mut lua_State, s: *const c_char, l: usize);
    #[link_name = "lua_pushstring"]
    pub fn lua_pushstring_(L: *mut lua_State, s: *const c_char);
    // lua_pushvfstring
    #[link_name = "lua_pushfstringL"]
    pub fn lua_pushfstring(L: *mut lua_State, fmt: *const c_char, ...) -> *const c_char;
    pub fn lua_pushcclosurek(
        L: *mut lua_State,
        f: lua_CFunction,
        debugname: *const c_char,
        nup: c_int,
        cont: Option<lua_Continuation>,
    );
    pub fn lua_pushboolean(L: *mut lua_State, b: c_int);
    pub fn lua_pushthread(L: *mut lua_State) -> c_int;

    pub fn lua_pushlightuserdatatagged(L: *mut lua_State, p: *mut c_void, tag: c_int);
    pub fn lua_newuserdatatagged(L: *mut lua_State, sz: usize, tag: c_int) -> *mut c_void;
    pub fn lua_newuserdatataggedwithmetatable(L: *mut lua_State, sz: usize, tag: c_int) -> *mut c_void;
    pub fn lua_newuserdatadtor(L: *mut lua_State, sz: usize, dtor: lua_Destructor) -> *mut c_void;

    pub fn lua_newbuffer(L: *mut lua_State, sz: usize) -> *mut c_void;

    //
    // Get functions (Lua -> stack)
    //
    pub fn lua_gettable(L: *mut lua_State, idx: c_int) -> c_int;
    pub fn lua_getfield(L: *mut lua_State, idx: c_int, k: *const c_char) -> c_int;
    pub fn lua_rawgetfield(L: *mut lua_State, idx: c_int, k: *const c_char) -> c_int;
    pub fn lua_rawget(L: *mut lua_State, idx: c_int) -> c_int;
    #[link_name = "lua_rawgeti"]
    pub fn lua_rawgeti_(L: *mut lua_State, idx: c_int, n: c_int) -> c_int;
    pub fn lua_createtable(L: *mut lua_State, narr: c_int, nrec: c_int);

    pub fn lua_setreadonly(L: *mut lua_State, idx: c_int, enabled: c_int);
    pub fn lua_getreadonly(L: *mut lua_State, idx: c_int) -> c_int;
    pub fn lua_setsafeenv(L: *mut lua_State, idx: c_int, enabled: c_int);

    pub fn lua_getmetatable(L: *mut lua_State, objindex: c_int) -> c_int;
    pub fn lua_getfenv(L: *mut lua_State, idx: c_int);

    //
    // Set functions (stack -> Lua)
    //
    pub fn lua_settable(L: *mut lua_State, idx: c_int);
    pub fn lua_setfield(L: *mut lua_State, idx: c_int, k: *const c_char);
    pub fn lua_rawset(L: *mut lua_State, idx: c_int);
    #[link_name = "lua_rawseti"]
    pub fn lua_rawseti_(L: *mut lua_State, idx: c_int, n: c_int);
    pub fn lua_setmetatable(L: *mut lua_State, objindex: c_int) -> c_int;
    pub fn lua_setfenv(L: *mut lua_State, idx: c_int) -> c_int;

    //
    // `load' and `call' functions (load and run Luau bytecode)
    //
    pub fn luau_load(
        L: *mut lua_State,
        chunkname: *const c_char,
        data: *const c_char,
        size: usize,
        env: c_int,
    ) -> c_int;
    pub fn lua_call(L: *mut lua_State, nargs: c_int, nresults: c_int);
    pub fn lua_pcall(L: *mut lua_State, nargs: c_int, nresults: c_int, errfunc: c_int) -> c_int;

    //
    // Coroutine functions
    //
    pub fn lua_yield(L: *mut lua_State, nresults: c_int) -> c_int;
    pub fn lua_break(L: *mut lua_State) -> c_int;
    #[link_name = "lua_resume"]
    pub fn lua_resume_(L: *mut lua_State, from: *mut lua_State, narg: c_int) -> c_int;
    pub fn lua_resumeerror(L: *mut lua_State, from: *mut lua_State) -> c_int;
    pub fn lua_status(L: *mut lua_State) -> c_int;
    pub fn lua_isyieldable(L: *mut lua_State) -> c_int;
    pub fn lua_getthreaddata(L: *mut lua_State) -> *mut c_void;
    pub fn lua_setthreaddata(L: *mut lua_State, data: *mut c_void);
}

//
// Garbage-collection function and options
//
pub const LUA_GCSTOP: c_int = 0;
pub const LUA_GCRESTART: c_int = 1;
pub const LUA_GCCOLLECT: c_int = 2;
pub const LUA_GCCOUNT: c_int = 3;
pub const LUA_GCCOUNTB: c_int = 4;
pub const LUA_GCISRUNNING: c_int = 5;
pub const LUA_GCSTEP: c_int = 6;
pub const LUA_GCSETGOAL: c_int = 7;
pub const LUA_GCSETSTEPMUL: c_int = 8;
pub const LUA_GCSETSTEPSIZE: c_int = 9;

unsafe extern "C-unwind" {
    pub fn lua_gc(L: *mut lua_State, what: c_int, data: c_int) -> c_int;
}

//
// Memory statistics
//
unsafe extern "C-unwind" {
    pub fn lua_setmemcat(L: *mut lua_State, category: c_int);
    pub fn lua_totalbytes(L: *mut lua_State, category: c_int) -> usize;
}

//
// Miscellaneous functions
//
unsafe extern "C-unwind" {
    pub fn lua_error(L: *mut lua_State) -> !;
    pub fn lua_next(L: *mut lua_State, idx: c_int) -> c_int;
    pub fn lua_rawiter(L: *mut lua_State, idx: c_int, iter: c_int) -> c_int;
    pub fn lua_concat(L: *mut lua_State, n: c_int);
    // TODO: lua_encodepointer
    pub fn lua_clock() -> c_double;
    pub fn lua_setuserdatatag(L: *mut lua_State, idx: c_int, tag: c_int);
    pub fn lua_setuserdatadtor(L: *mut lua_State, tag: c_int, dtor: Option<lua_Destructor>);
    pub fn lua_getuserdatadtor(L: *mut lua_State, tag: c_int) -> Option<lua_Destructor>;
    pub fn lua_setuserdatametatable(L: *mut lua_State, tag: c_int);
    pub fn lua_getuserdatametatable(L: *mut lua_State, tag: c_int);
    pub fn lua_setlightuserdataname(L: *mut lua_State, tag: c_int, name: *const c_char);
    pub fn lua_getlightuserdataname(L: *mut lua_State, tag: c_int) -> *const c_char;
    pub fn lua_clonefunction(L: *mut lua_State, idx: c_int);
    pub fn lua_cleartable(L: *mut lua_State, idx: c_int);
    pub fn lua_getallocf(L: *mut lua_State, ud: *mut *mut c_void) -> lua_Alloc;
}

//
// Reference system, can be used to pin objects
//
pub const LUA_NOREF: c_int = -1;
pub const LUA_REFNIL: c_int = 0;

unsafe extern "C-unwind" {
    pub fn lua_ref(L: *mut lua_State, idx: c_int) -> c_int;
    pub fn lua_unref(L: *mut lua_State, r#ref: c_int);
}

//
// Some useful macros (implemented as Rust functions)
//

#[inline(always)]
pub unsafe fn lua_tonumber(L: *mut lua_State, idx: c_int) -> lua_Number {
    lua_tonumberx(L, idx, ptr::null_mut())
}

#[inline(always)]
pub unsafe fn lua_tointeger_(L: *mut lua_State, idx: c_int) -> c_int {
    lua_tointegerx_(L, idx, ptr::null_mut())
}

#[inline(always)]
pub unsafe fn lua_tounsigned(L: *mut lua_State, i: c_int) -> lua_Unsigned {
    lua_tounsignedx(L, i, ptr::null_mut())
}

#[inline(always)]
pub unsafe fn lua_pop(L: *mut lua_State, n: c_int) {
    lua_settop(L, -n - 1)
}

#[inline(always)]
pub unsafe fn lua_newtable(L: *mut lua_State) {
    lua_createtable(L, 0, 0)
}

#[inline(always)]
pub unsafe fn lua_newuserdata(L: *mut lua_State, sz: usize) -> *mut c_void {
    lua_newuserdatatagged(L, sz, 0)
}

#[inline(always)]
pub unsafe fn lua_newuserdata_t<T>(L: *mut lua_State, data: T) -> *mut T {
    unsafe extern "C" fn destructor<T>(_: *mut lua_State, ud: *mut c_void) {
        ptr::drop_in_place(ud as *mut T);
    }

    let ud_ptr = lua_newuserdatadtor(L, const { mem::size_of::<T>() }, destructor::<T>) as *mut T;
    ptr::write(ud_ptr, data);
    ud_ptr
}

// TODO: lua_strlen

#[inline(always)]
pub unsafe fn lua_isfunction(L: *mut lua_State, n: c_int) -> c_int {
    (lua_type(L, n) == LUA_TFUNCTION) as c_int
}

#[inline(always)]
pub unsafe fn lua_istable(L: *mut lua_State, n: c_int) -> c_int {
    (lua_type(L, n) == LUA_TTABLE) as c_int
}

#[inline(always)]
pub unsafe fn lua_islightuserdata(L: *mut lua_State, n: c_int) -> c_int {
    (lua_type(L, n) == LUA_TLIGHTUSERDATA) as c_int
}

#[inline(always)]
pub unsafe fn lua_isnil(L: *mut lua_State, n: c_int) -> c_int {
    (lua_type(L, n) == LUA_TNIL) as c_int
}

#[inline(always)]
pub unsafe fn lua_isboolean(L: *mut lua_State, n: c_int) -> c_int {
    (lua_type(L, n) == LUA_TBOOLEAN) as c_int
}

#[inline(always)]
pub unsafe fn lua_isvector(L: *mut lua_State, n: c_int) -> c_int {
    (lua_type(L, n) == LUA_TVECTOR) as c_int
}

#[inline(always)]
pub unsafe fn lua_isthread(L: *mut lua_State, n: c_int) -> c_int {
    (lua_type(L, n) == LUA_TTHREAD) as c_int
}

#[inline(always)]
pub unsafe fn lua_isbuffer(L: *mut lua_State, n: c_int) -> c_int {
    (lua_type(L, n) == LUA_TBUFFER) as c_int
}

#[inline(always)]
pub unsafe fn lua_isnone(L: *mut lua_State, n: c_int) -> c_int {
    (lua_type(L, n) == LUA_TNONE) as c_int
}

#[inline(always)]
pub unsafe fn lua_isnoneornil(L: *mut lua_State, n: c_int) -> c_int {
    (lua_type(L, n) <= LUA_TNIL) as c_int
}

#[inline(always)]
pub unsafe fn lua_pushliteral(L: *mut lua_State, s: &'static CStr) {
    lua_pushstring_(L, s.as_ptr());
}

#[inline(always)]
pub unsafe fn lua_pushcfunction(L: *mut lua_State, f: lua_CFunction) {
    lua_pushcclosurek(L, f, ptr::null(), 0, None)
}

#[inline(always)]
pub unsafe fn lua_pushcfunctiond(L: *mut lua_State, f: lua_CFunction, debugname: *const c_char) {
    lua_pushcclosurek(L, f, debugname, 0, None)
}

#[inline(always)]
pub unsafe fn lua_pushcclosure(L: *mut lua_State, f: lua_CFunction, nup: c_int) {
    lua_pushcclosurek(L, f, ptr::null(), nup, None)
}

#[inline(always)]
pub unsafe fn lua_pushcclosured(L: *mut lua_State, f: lua_CFunction, debugname: *const c_char, nup: c_int) {
    lua_pushcclosurek(L, f, debugname, nup, None)
}

#[inline(always)]
pub unsafe fn lua_pushlightuserdata(L: *mut lua_State, p: *mut c_void) {
    lua_pushlightuserdatatagged(L, p, 0)
}

#[inline(always)]
pub unsafe fn lua_setglobal(L: *mut lua_State, var: *const c_char) {
    lua_setfield(L, LUA_GLOBALSINDEX, var)
}

#[inline(always)]
pub unsafe fn lua_getglobal(L: *mut lua_State, var: *const c_char) -> c_int {
    lua_getfield(L, LUA_GLOBALSINDEX, var)
}

#[inline(always)]
pub unsafe fn lua_tostring(L: *mut lua_State, i: c_int) -> *const c_char {
    lua_tolstring(L, i, ptr::null_mut())
}

//
// Debug API
//

// Maximum size for the description of the source of a function in debug information.
const LUA_IDSIZE: usize = 256;

/// Type for functions to be called on debug events.
pub type lua_Hook = unsafe extern "C-unwind" fn(L: *mut lua_State, ar: *mut lua_Debug);

pub type lua_Coverage = unsafe extern "C-unwind" fn(
    context: *mut c_void,
    function: *const c_char,
    linedefined: c_int,
    depth: c_int,
    hits: *const c_int,
    size: usize,
);

unsafe extern "C-unwind" {
    pub fn lua_stackdepth(L: *mut lua_State) -> c_int;
    pub fn lua_getinfo(L: *mut lua_State, level: c_int, what: *const c_char, ar: *mut lua_Debug) -> c_int;
    pub fn lua_getargument(L: *mut lua_State, level: c_int, n: c_int) -> c_int;
    pub fn lua_getlocal(L: *mut lua_State, level: c_int, n: c_int) -> *const c_char;
    pub fn lua_setlocal(L: *mut lua_State, level: c_int, n: c_int) -> *const c_char;
    pub fn lua_getupvalue(L: *mut lua_State, funcindex: c_int, n: c_int) -> *const c_char;
    pub fn lua_setupvalue(L: *mut lua_State, funcindex: c_int, n: c_int) -> *const c_char;

    pub fn lua_singlestep(L: *mut lua_State, enabled: c_int);
    pub fn lua_breakpoint(L: *mut lua_State, funcindex: c_int, line: c_int, enabled: c_int) -> c_int;

    pub fn lua_getcoverage(L: *mut lua_State, funcindex: c_int, context: *mut c_void, callback: lua_Coverage);

    pub fn lua_debugtrace(L: *mut lua_State) -> *const c_char;
}

#[repr(C)]
pub struct lua_Debug {
    pub name: *const c_char,
    pub what: *const c_char,
    pub source: *const c_char,
    pub short_src: *const c_char,
    pub linedefined: c_int,
    pub currentline: c_int,
    pub nupvals: u8,
    pub nparams: u8,
    pub isvararg: c_char,
    pub userdata: *mut c_void,
    pub ssbuf: [c_char; LUA_IDSIZE],
}

//
// Callbacks that can be used to reconfigure behavior of the VM dynamically.
// These are shared between all coroutines.
//

#[repr(C)]
#[non_exhaustive]
pub struct lua_Callbacks {
    /// arbitrary userdata pointer that is never overwritten by Luau
    pub userdata: *mut c_void,

    /// gets called at safepoints (loop back edges, call/ret, gc) if set
    pub interrupt: Option<unsafe extern "C-unwind" fn(L: *mut lua_State, gc: c_int)>,
    /// gets called when an unprotected error is raised (if longjmp is used)
    pub panic: Option<unsafe extern "C-unwind" fn(L: *mut lua_State, errcode: c_int)>,

    /// gets called when L is created (LP == parent) or destroyed (LP == NULL)
    pub userthread: Option<unsafe extern "C-unwind" fn(LP: *mut lua_State, L: *mut lua_State)>,
    /// gets called when a string is created; returned atom can be retrieved via tostringatom
    pub useratom: Option<unsafe extern "C-unwind" fn(s: *const c_char, l: usize) -> i16>,

    /// gets called when BREAK instruction is encountered
    pub debugbreak: Option<unsafe extern "C-unwind" fn(L: *mut lua_State, ar: *mut lua_Debug)>,
    /// gets called after each instruction in single step mode
    pub debugstep: Option<unsafe extern "C-unwind" fn(L: *mut lua_State, ar: *mut lua_Debug)>,
    /// gets called when thread execution is interrupted by break in another thread
    pub debuginterrupt: Option<unsafe extern "C-unwind" fn(L: *mut lua_State, ar: *mut lua_Debug)>,
    /// gets called when protected call results in an error
    pub debugprotectederror: Option<unsafe extern "C-unwind" fn(L: *mut lua_State)>,

    /// gets called when memory is allocated
    pub onallocate: Option<unsafe extern "C-unwind" fn(L: *mut lua_State, osize: usize, nsize: usize)>,
}

unsafe extern "C" {
    pub fn lua_callbacks(L: *mut lua_State) -> *mut lua_Callbacks;
}

// Functions from customization lib
unsafe extern "C" {
    pub fn luau_setfflag(name: *const c_char, value: c_int) -> c_int;
    pub fn lua_getmetatablepointer(L: *mut lua_State, idx: c_int) -> *const c_void;
}
