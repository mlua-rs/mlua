use std::ffi::CStr;
use std::os::raw::{c_int, c_void};
use std::{ptr, str};

use crate::error::Result;
use crate::util::{check_stack, get_metatable_ptr, push_table, rawget_field, rawset_field, TypeKey};

// Pushes the userdata and attaches a metatable with __gc method.
// Internally uses 3 stack spaces, does not call checkstack.
pub(crate) unsafe fn push_internal_userdata<T: TypeKey>(
    state: *mut ffi::lua_State,
    t: T,
    protect: bool,
) -> Result<()> {
    push_userdata(state, t, protect)?;
    get_internal_metatable::<T>(state);
    ffi::lua_setmetatable(state, -2);
    Ok(())
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
    customize_fn: Option<fn(*mut ffi::lua_State) -> Result<()>>,
) -> Result<()> {
    check_stack(state, 6)?;

    push_table(state, 0, 3, true)?;

    #[cfg(not(feature = "luau"))]
    {
        ffi::lua_pushcfunction(state, userdata_destructor::<T>);
        rawset_field(state, -2, "__gc")?;
    }

    ffi::lua_pushboolean(state, 0);
    rawset_field(state, -2, "__metatable")?;

    if let Some(f) = customize_fn {
        f(state)?;
    }

    protect_lua!(state, 1, 0, |state| {
        ffi::lua_rawsetp(state, ffi::LUA_REGISTRYINDEX, T::type_key());
    })?;

    Ok(())
}

// Uses 2 stack spaces, does not call checkstack
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
            ffi::lua_newuserdata(state, std::mem::size_of::<T>()) as *mut T
        })
    } else {
        Ok(ffi::lua_newuserdata(state, std::mem::size_of::<T>()) as *mut T)
    }
}

// Internally uses 3 stack spaces, does not call checkstack.
#[inline]
pub(crate) unsafe fn push_userdata<T>(state: *mut ffi::lua_State, t: T, protect: bool) -> Result<*mut T> {
    #[cfg(not(feature = "luau"))]
    let ud_ptr = push_uninit_userdata(state, protect)?;
    #[cfg(feature = "luau")]
    let ud_ptr = if protect {
        protect_lua!(state, 0, 1, |state| { ffi::lua_newuserdata_t::<T>(state) })?
    } else {
        ffi::lua_newuserdata_t::<T>(state)
    };
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

// Pops the userdata off of the top of the stack and returns it to rust, invalidating the lua
// userdata and gives it the special "destructed" userdata metatable. Userdata must not have been
// previously invalidated, and this method does not check for this.
// Uses 1 extra stack space and does not call checkstack.
pub(crate) unsafe fn take_userdata<T>(state: *mut ffi::lua_State) -> T {
    // We set the metatable of userdata on __gc to a special table with no __gc method and with
    // metamethods that trigger an error on access. We do this so that it will not be double
    // dropped, and also so that it cannot be used or identified as any particular userdata type
    // after the first call to __gc.
    get_destructed_userdata_metatable(state);
    ffi::lua_setmetatable(state, -2);
    let ud = get_userdata::<T>(state, -1);

    // Update userdata tag to disable destructor and mark as destructed
    #[cfg(feature = "luau")]
    ffi::lua_setuserdatatag(state, -1, 1);

    ffi::lua_pop(state, 1);
    ptr::read(ud)
}

pub(crate) unsafe fn get_destructed_userdata_metatable(state: *mut ffi::lua_State) {
    let key = &DESTRUCTED_USERDATA_METATABLE as *const u8 as *const c_void;
    ffi::lua_rawgetp(state, ffi::LUA_REGISTRYINDEX, key);
}

// Populates the given table with the appropriate members to be a userdata metatable for the given
// type. This function takes the given table at the `metatable` index, and adds an appropriate
// `__gc` member to it for the given type and a `__metatable` entry to protect the table from script
// access. The function also, if given a `field_getters` or `methods` tables, will create an
// `__index` metamethod (capturing previous one) to lookup in `field_getters` first, then `methods`
// and falling back to the captured `__index` if no matches found.
// The same is also applicable for `__newindex` metamethod and `field_setters` table.
// Internally uses 9 stack spaces and does not call checkstack.
pub(crate) unsafe fn init_userdata_metatable(
    state: *mut ffi::lua_State,
    metatable: c_int,
    field_getters: Option<c_int>,
    field_setters: Option<c_int>,
    methods: Option<c_int>,
) -> Result<()> {
    if field_getters.is_some() || methods.is_some() {
        // Push `__index` generator function
        init_userdata_metatable_index(state)?;

        let index_type = rawget_field(state, metatable, "__index")?;
        match index_type {
            ffi::LUA_TNIL | ffi::LUA_TTABLE | ffi::LUA_TFUNCTION => {
                for &idx in &[field_getters, methods] {
                    if let Some(idx) = idx {
                        ffi::lua_pushvalue(state, idx);
                    } else {
                        ffi::lua_pushnil(state);
                    }
                }

                // Generate `__index`
                protect_lua!(state, 4, 1, fn(state) ffi::lua_call(state, 3, 1))?;
            }
            _ => mlua_panic!("improper `__index` type: {}", index_type),
        }

        rawset_field(state, metatable, "__index")?;
    }

    if let Some(field_setters) = field_setters {
        // Push `__newindex` generator function
        init_userdata_metatable_newindex(state)?;

        let newindex_type = rawget_field(state, metatable, "__newindex")?;
        match newindex_type {
            ffi::LUA_TNIL | ffi::LUA_TTABLE | ffi::LUA_TFUNCTION => {
                ffi::lua_pushvalue(state, field_setters);
                // Generate `__newindex`
                protect_lua!(state, 3, 1, fn(state) ffi::lua_call(state, 2, 1))?;
            }
            _ => mlua_panic!("improper `__newindex` type: {}", newindex_type),
        }

        rawset_field(state, metatable, "__newindex")?;
    }

    ffi::lua_pushboolean(state, 0);
    rawset_field(state, metatable, "__metatable")?;

    Ok(())
}

unsafe extern "C-unwind" fn lua_error_impl(state: *mut ffi::lua_State) -> c_int {
    ffi::lua_error(state);
}

unsafe extern "C-unwind" fn lua_isfunction_impl(state: *mut ffi::lua_State) -> c_int {
    ffi::lua_pushboolean(state, ffi::lua_isfunction(state, -1));
    1
}

unsafe extern "C-unwind" fn lua_istable_impl(state: *mut ffi::lua_State) -> c_int {
    ffi::lua_pushboolean(state, ffi::lua_istable(state, -1));
    1
}

unsafe fn init_userdata_metatable_index(state: *mut ffi::lua_State) -> Result<()> {
    let index_key = &USERDATA_METATABLE_INDEX as *const u8 as *const _;
    if ffi::lua_rawgetp(state, ffi::LUA_REGISTRYINDEX, index_key) == ffi::LUA_TFUNCTION {
        return Ok(());
    }
    ffi::lua_pop(state, 1);

    // Create and cache `__index` generator
    let code = cstr!(
        r#"
            local error, isfunction, istable = ...
            return function (__index, field_getters, methods)
                -- Common case: has field getters and index is a table
                if field_getters ~= nil and methods == nil and istable(__index) then
                    return function (self, key)
                        local field_getter = field_getters[key]
                        if field_getter ~= nil then
                            return field_getter(self)
                        end
                        return __index[key]
                    end
                end

                return function (self, key)
                    if field_getters ~= nil then
                        local field_getter = field_getters[key]
                        if field_getter ~= nil then
                            return field_getter(self)
                        end
                    end

                    if methods ~= nil then
                        local method = methods[key]
                        if method ~= nil then
                            return method
                        end
                    end

                    if isfunction(__index) then
                        return __index(self, key)
                    elseif __index == nil then
                        error("attempt to get an unknown field '"..key.."'")
                    else
                        return __index[key]
                    end
                end
            end
    "#
    );
    let code_len = CStr::from_ptr(code).to_bytes().len();
    protect_lua!(state, 0, 1, |state| {
        let ret = ffi::luaL_loadbuffer(state, code, code_len, cstr!("__mlua_index"));
        if ret != ffi::LUA_OK {
            ffi::lua_error(state);
        }
        ffi::lua_pushcfunction(state, lua_error_impl);
        ffi::lua_pushcfunction(state, lua_isfunction_impl);
        ffi::lua_pushcfunction(state, lua_istable_impl);
        ffi::lua_call(state, 3, 1);

        #[cfg(feature = "luau-jit")]
        if ffi::luau_codegen_supported() != 0 {
            ffi::luau_codegen_compile(state, -1);
        }

        // Store in the registry
        ffi::lua_pushvalue(state, -1);
        ffi::lua_rawsetp(state, ffi::LUA_REGISTRYINDEX, index_key);
    })
}

unsafe fn init_userdata_metatable_newindex(state: *mut ffi::lua_State) -> Result<()> {
    let newindex_key = &USERDATA_METATABLE_NEWINDEX as *const u8 as *const _;
    if ffi::lua_rawgetp(state, ffi::LUA_REGISTRYINDEX, newindex_key) == ffi::LUA_TFUNCTION {
        return Ok(());
    }
    ffi::lua_pop(state, 1);

    // Create and cache `__newindex` generator
    let code = cstr!(
        r#"
            local error, isfunction = ...
            return function (__newindex, field_setters)
                return function (self, key, value)
                    if field_setters ~= nil then
                        local field_setter = field_setters[key]
                        if field_setter ~= nil then
                            field_setter(self, value)
                            return
                        end
                    end

                    if isfunction(__newindex) then
                        __newindex(self, key, value)
                    elseif __newindex == nil then
                        error("attempt to set an unknown field '"..key.."'")
                    else
                        __newindex[key] = value
                    end
                end
            end
    "#
    );
    let code_len = CStr::from_ptr(code).to_bytes().len();
    protect_lua!(state, 0, 1, |state| {
        let ret = ffi::luaL_loadbuffer(state, code, code_len, cstr!("__mlua_newindex"));
        if ret != ffi::LUA_OK {
            ffi::lua_error(state);
        }
        ffi::lua_pushcfunction(state, lua_error_impl);
        ffi::lua_pushcfunction(state, lua_isfunction_impl);
        ffi::lua_call(state, 2, 1);

        #[cfg(feature = "luau-jit")]
        if ffi::luau_codegen_supported() != 0 {
            ffi::luau_codegen_compile(state, -1);
        }

        // Store in the registry
        ffi::lua_pushvalue(state, -1);
        ffi::lua_rawsetp(state, ffi::LUA_REGISTRYINDEX, newindex_key);
    })
}

#[cfg(not(feature = "luau"))]
unsafe extern "C-unwind" fn userdata_destructor<T>(state: *mut ffi::lua_State) -> c_int {
    // It's probably NOT a good idea to catch Rust panics in finalizer
    // Lua 5.4 ignores it, other versions generates `LUA_ERRGCMM` without calling message handler
    take_userdata::<T>(state);
    0
}

pub(crate) static DESTRUCTED_USERDATA_METATABLE: u8 = 0;
static USERDATA_METATABLE_INDEX: u8 = 0;
static USERDATA_METATABLE_NEWINDEX: u8 = 0;
