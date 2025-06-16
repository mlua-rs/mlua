use std::os::raw::{c_int, c_void};
use std::{mem, ptr};

use crate::error::Result;
use crate::userdata::collect_userdata;
use crate::util::{check_stack, get_metatable_ptr, push_table, rawset_field, TypeKey};

// Pushes the userdata and attaches a metatable with __gc method.
// Internally uses 3 stack spaces, does not call checkstack.
pub(crate) unsafe fn push_internal_userdata<T: TypeKey>(
    state: *mut ffi::lua_State,
    t: T,
    protect: bool,
) -> Result<*mut T> {
    #[cfg(not(feature = "luau"))]
    let ud_ptr = if protect {
        protect_lua!(state, 0, 1, move |state| {
            let ud_ptr = ffi::lua_newuserdata(state, const { mem::size_of::<T>() }) as *mut T;
            ptr::write(ud_ptr, t);
            ud_ptr
        })?
    } else {
        let ud_ptr = ffi::lua_newuserdata(state, const { mem::size_of::<T>() }) as *mut T;
        ptr::write(ud_ptr, t);
        ud_ptr
    };

    #[cfg(feature = "luau")]
    let ud_ptr = if protect {
        protect_lua!(state, 0, 1, move |state| ffi::lua_newuserdata_t::<T>(state, t))?
    } else {
        ffi::lua_newuserdata_t::<T>(state, t)
    };

    get_internal_metatable::<T>(state);
    ffi::lua_setmetatable(state, -2);
    Ok(ud_ptr)
}

#[track_caller]
pub(crate) unsafe fn get_internal_metatable<T: TypeKey>(state: *mut ffi::lua_State) {
    ffi::lua_rawgetp(state, ffi::LUA_REGISTRYINDEX, T::type_key());
    debug_assert!(ffi::lua_isnil(state, -1) == 0, "internal metatable not found");
}

// Initialize the internal metatable for a type T (with __gc method).
// Uses 6 stack spaces and calls checkstack.
pub(crate) unsafe fn init_internal_metatable<T: TypeKey>(
    state: *mut ffi::lua_State,
    customize_fn: Option<fn(*mut ffi::lua_State)>,
) -> Result<()> {
    check_stack(state, 6)?;

    push_table(state, 0, 3, true)?;

    #[cfg(not(feature = "luau"))]
    {
        ffi::lua_pushcfunction(state, collect_userdata::<T>);
        rawset_field(state, -2, "__gc")?;
    }

    ffi::lua_pushboolean(state, 0);
    rawset_field(state, -2, "__metatable")?;

    protect_lua!(state, 1, 0, |state| {
        if let Some(f) = customize_fn {
            f(state);
        }

        ffi::lua_rawsetp(state, ffi::LUA_REGISTRYINDEX, T::type_key());
    })?;

    Ok(())
}

// Uses up to 1 stack space, does not call `checkstack`
pub(crate) unsafe fn get_internal_userdata<T: TypeKey>(
    state: *mut ffi::lua_State,
    index: c_int,
    mut type_mt_ptr: *const c_void,
) -> *mut T {
    let ud = ffi::lua_touserdata(state, index) as *mut T;
    if ud.is_null() {
        return ptr::null_mut();
    }
    let mt_ptr = get_metatable_ptr(state, index);
    if type_mt_ptr.is_null() {
        get_internal_metatable::<T>(state);
        type_mt_ptr = ffi::lua_topointer(state, -1);
        ffi::lua_pop(state, 1);
    }
    if mt_ptr != type_mt_ptr {
        return ptr::null_mut();
    }
    ud
}

// Internally uses 3 stack spaces, does not call checkstack.
#[inline]
#[cfg(not(feature = "luau"))]
pub(crate) unsafe fn push_uninit_userdata<T>(state: *mut ffi::lua_State, protect: bool) -> Result<*mut T> {
    if protect {
        protect_lua!(state, 0, 1, |state| {
            ffi::lua_newuserdata(state, const { mem::size_of::<T>() }) as *mut T
        })
    } else {
        Ok(ffi::lua_newuserdata(state, const { mem::size_of::<T>() }) as *mut T)
    }
}

// Internally uses 3 stack spaces, does not call checkstack.
#[inline]
pub(crate) unsafe fn push_userdata<T>(state: *mut ffi::lua_State, t: T, protect: bool) -> Result<*mut T> {
    let size = const { mem::size_of::<T>() };

    #[cfg(not(feature = "luau"))]
    let ud_ptr = if protect {
        protect_lua!(state, 0, 1, move |state| ffi::lua_newuserdata(state, size))?
    } else {
        ffi::lua_newuserdata(state, size)
    } as *mut T;

    #[cfg(feature = "luau")]
    let ud_ptr = if protect {
        protect_lua!(state, 0, 1, |state| {
            ffi::lua_newuserdatadtor(state, size, collect_userdata::<T>)
        })?
    } else {
        ffi::lua_newuserdatadtor(state, size, collect_userdata::<T>)
    } as *mut T;

    ptr::write(ud_ptr, t);
    Ok(ud_ptr)
}

#[inline]
#[track_caller]
pub(crate) unsafe fn get_userdata<T>(state: *mut ffi::lua_State, index: c_int) -> *mut T {
    let ud = ffi::lua_touserdata(state, index) as *mut T;
    mlua_debug_assert!(!ud.is_null(), "userdata pointer is null");
    ud
}

/// Unwraps `T` from the Lua userdata and invalidating it by setting the special "destructed"
/// metatable.
///
/// This method does not check that userdata is of type `T` and was not previously invalidated.
///
/// Uses 1 extra stack space, does not call checkstack.
pub(crate) unsafe fn take_userdata<T>(state: *mut ffi::lua_State, idx: c_int) -> T {
    #[rustfmt::skip]
    let idx = if idx < 0 { ffi::lua_absindex(state, idx) } else { idx };

    // Update the metatable of this userdata to a special one with no `__gc` method and with
    // metamethods that trigger an error on access.
    // We do this so that it will not be double dropped or used after being dropped.
    get_destructed_userdata_metatable(state);
    ffi::lua_setmetatable(state, idx);
    let ud = get_userdata::<T>(state, idx);

    // Update userdata tag to disable destructor and mark as destructed
    #[cfg(feature = "luau")]
    ffi::lua_setuserdatatag(state, idx, 1);

    ptr::read(ud)
}

pub(crate) unsafe fn get_destructed_userdata_metatable(state: *mut ffi::lua_State) {
    let key = &DESTRUCTED_USERDATA_METATABLE as *const u8 as *const c_void;
    ffi::lua_rawgetp(state, ffi::LUA_REGISTRYINDEX, key);
}

pub(crate) static DESTRUCTED_USERDATA_METATABLE: u8 = 0;
