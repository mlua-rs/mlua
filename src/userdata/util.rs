use std::any::TypeId;
use std::cell::Cell;
use std::marker::PhantomData;
use std::os::raw::c_int;
use std::ptr;

use super::UserDataStorage;
use crate::error::{Error, Result};
use crate::util::{get_userdata, rawget_field, rawset_field, take_userdata};

// This is a trick to check if a type is `Sync` or not.
// It uses leaked specialization feature from stdlib.
struct IsSync<'a, T> {
    is_sync: &'a Cell<bool>,
    _marker: PhantomData<T>,
}

impl<T> Clone for IsSync<'_, T> {
    fn clone(&self) -> Self {
        self.is_sync.set(false);
        IsSync {
            is_sync: self.is_sync,
            _marker: PhantomData,
        }
    }
}

impl<T: Sync> Copy for IsSync<'_, T> {}

pub(crate) fn is_sync<T>() -> bool {
    let is_sync = Cell::new(true);
    let _ = [IsSync::<T> {
        is_sync: &is_sync,
        _marker: PhantomData,
    }]
    .clone();
    is_sync.get()
}

// Userdata type hints,  used to match types of wrapped userdata
#[derive(Clone, Copy)]
pub(crate) struct TypeIdHints {
    t: TypeId,

    #[cfg(all(feature = "userdata-wrappers", not(feature = "send")))]
    rc: TypeId,
    #[cfg(all(feature = "userdata-wrappers", not(feature = "send")))]
    rc_refcell: TypeId,

    #[cfg(feature = "userdata-wrappers")]
    arc: TypeId,
    #[cfg(feature = "userdata-wrappers")]
    arc_mutex: TypeId,
    #[cfg(feature = "userdata-wrappers")]
    arc_rwlock: TypeId,
    #[cfg(feature = "userdata-wrappers")]
    arc_pl_mutex: TypeId,
    #[cfg(feature = "userdata-wrappers")]
    arc_pl_rwlock: TypeId,
}

impl TypeIdHints {
    pub(crate) fn new<T: 'static>() -> Self {
        Self {
            t: TypeId::of::<T>(),

            #[cfg(all(feature = "userdata-wrappers", not(feature = "send")))]
            rc: TypeId::of::<std::rc::Rc<T>>(),
            #[cfg(all(feature = "userdata-wrappers", not(feature = "send")))]
            rc_refcell: TypeId::of::<std::rc::Rc<std::cell::RefCell<T>>>(),

            #[cfg(feature = "userdata-wrappers")]
            arc: TypeId::of::<std::sync::Arc<T>>(),
            #[cfg(feature = "userdata-wrappers")]
            arc_mutex: TypeId::of::<std::sync::Arc<std::sync::Mutex<T>>>(),
            #[cfg(feature = "userdata-wrappers")]
            arc_rwlock: TypeId::of::<std::sync::Arc<std::sync::RwLock<T>>>(),
            #[cfg(feature = "userdata-wrappers")]
            arc_pl_mutex: TypeId::of::<std::sync::Arc<parking_lot::Mutex<T>>>(),
            #[cfg(feature = "userdata-wrappers")]
            arc_pl_rwlock: TypeId::of::<std::sync::Arc<parking_lot::RwLock<T>>>(),
        }
    }

    #[inline(always)]
    pub(crate) fn type_id(&self) -> TypeId {
        self.t
    }
}

pub(crate) unsafe fn borrow_userdata_scoped<T, R>(
    state: *mut ffi::lua_State,
    idx: c_int,
    type_id: Option<TypeId>,
    type_hints: TypeIdHints,
    f: impl FnOnce(&T) -> R,
) -> Result<R> {
    match type_id {
        Some(type_id) if type_id == type_hints.t => {
            let ud = get_userdata::<UserDataStorage<T>>(state, idx);
            (*ud).try_borrow_scoped(|ud| f(ud))
        }

        #[cfg(all(feature = "userdata-wrappers", not(feature = "send")))]
        Some(type_id) if type_id == type_hints.rc => {
            let ud = get_userdata::<UserDataStorage<std::rc::Rc<T>>>(state, idx);
            (*ud).try_borrow_scoped(|ud| f(ud))
        }
        #[cfg(all(feature = "userdata-wrappers", not(feature = "send")))]
        Some(type_id) if type_id == type_hints.rc_refcell => {
            let ud = get_userdata::<UserDataStorage<std::rc::Rc<std::cell::RefCell<T>>>>(state, idx);
            (*ud).try_borrow_scoped(|ud| {
                let ud = ud.try_borrow().map_err(|_| Error::UserDataBorrowError)?;
                Ok(f(&ud))
            })?
        }

        #[cfg(feature = "userdata-wrappers")]
        Some(type_id) if type_id == type_hints.arc => {
            let ud = get_userdata::<UserDataStorage<std::sync::Arc<T>>>(state, idx);
            (*ud).try_borrow_scoped(|ud| f(ud))
        }
        #[cfg(feature = "userdata-wrappers")]
        Some(type_id) if type_id == type_hints.arc_mutex => {
            let ud = get_userdata::<UserDataStorage<std::sync::Arc<std::sync::Mutex<T>>>>(state, idx);
            (*ud).try_borrow_scoped(|ud| {
                let ud = ud.try_lock().map_err(|_| Error::UserDataBorrowError)?;
                Ok(f(&ud))
            })?
        }
        #[cfg(feature = "userdata-wrappers")]
        Some(type_id) if type_id == type_hints.arc_rwlock => {
            let ud = get_userdata::<UserDataStorage<std::sync::Arc<std::sync::RwLock<T>>>>(state, idx);
            (*ud).try_borrow_scoped(|ud| {
                let ud = ud.try_read().map_err(|_| Error::UserDataBorrowError)?;
                Ok(f(&ud))
            })?
        }
        #[cfg(feature = "userdata-wrappers")]
        Some(type_id) if type_id == type_hints.arc_pl_mutex => {
            let ud = get_userdata::<UserDataStorage<std::sync::Arc<parking_lot::Mutex<T>>>>(state, idx);
            (*ud).try_borrow_scoped(|ud| {
                let ud = ud.try_lock().ok_or(Error::UserDataBorrowError)?;
                Ok(f(&ud))
            })?
        }
        #[cfg(feature = "userdata-wrappers")]
        Some(type_id) if type_id == type_hints.arc_pl_rwlock => {
            let ud = get_userdata::<UserDataStorage<std::sync::Arc<parking_lot::RwLock<T>>>>(state, idx);
            (*ud).try_borrow_scoped(|ud| {
                let ud = ud.try_read().ok_or(Error::UserDataBorrowError)?;
                Ok(f(&ud))
            })?
        }
        _ => Err(Error::UserDataTypeMismatch),
    }
}

pub(crate) unsafe fn borrow_userdata_scoped_mut<T, R>(
    state: *mut ffi::lua_State,
    idx: c_int,
    type_id: Option<TypeId>,
    type_hints: TypeIdHints,
    f: impl FnOnce(&mut T) -> R,
) -> Result<R> {
    match type_id {
        Some(type_id) if type_id == type_hints.t => {
            let ud = get_userdata::<UserDataStorage<T>>(state, idx);
            (*ud).try_borrow_scoped_mut(|ud| f(ud))
        }

        #[cfg(all(feature = "userdata-wrappers", not(feature = "send")))]
        Some(type_id) if type_id == type_hints.rc => {
            let ud = get_userdata::<UserDataStorage<std::rc::Rc<T>>>(state, idx);
            (*ud).try_borrow_scoped_mut(|ud| match std::rc::Rc::get_mut(ud) {
                Some(ud) => Ok(f(ud)),
                None => Err(Error::UserDataBorrowMutError),
            })?
        }
        #[cfg(all(feature = "userdata-wrappers", not(feature = "send")))]
        Some(type_id) if type_id == type_hints.rc_refcell => {
            let ud = get_userdata::<UserDataStorage<std::rc::Rc<std::cell::RefCell<T>>>>(state, idx);
            (*ud).try_borrow_scoped(|ud| {
                let mut ud = ud.try_borrow_mut().map_err(|_| Error::UserDataBorrowMutError)?;
                Ok(f(&mut ud))
            })?
        }

        #[cfg(feature = "userdata-wrappers")]
        Some(type_id) if type_id == type_hints.arc => {
            let ud = get_userdata::<UserDataStorage<std::sync::Arc<T>>>(state, idx);
            (*ud).try_borrow_scoped_mut(|ud| match std::sync::Arc::get_mut(ud) {
                Some(ud) => Ok(f(ud)),
                None => Err(Error::UserDataBorrowMutError),
            })?
        }
        #[cfg(feature = "userdata-wrappers")]
        Some(type_id) if type_id == type_hints.arc_mutex => {
            let ud = get_userdata::<UserDataStorage<std::sync::Arc<std::sync::Mutex<T>>>>(state, idx);
            (*ud).try_borrow_scoped_mut(|ud| {
                let mut ud = ud.try_lock().map_err(|_| Error::UserDataBorrowMutError)?;
                Ok(f(&mut ud))
            })?
        }
        #[cfg(feature = "userdata-wrappers")]
        Some(type_id) if type_id == type_hints.arc_rwlock => {
            let ud = get_userdata::<UserDataStorage<std::sync::Arc<std::sync::RwLock<T>>>>(state, idx);
            (*ud).try_borrow_scoped_mut(|ud| {
                let mut ud = ud.try_write().map_err(|_| Error::UserDataBorrowMutError)?;
                Ok(f(&mut ud))
            })?
        }
        #[cfg(feature = "userdata-wrappers")]
        Some(type_id) if type_id == type_hints.arc_pl_mutex => {
            let ud = get_userdata::<UserDataStorage<std::sync::Arc<parking_lot::Mutex<T>>>>(state, idx);
            (*ud).try_borrow_scoped_mut(|ud| {
                let mut ud = ud.try_lock().ok_or(Error::UserDataBorrowMutError)?;
                Ok(f(&mut ud))
            })?
        }
        #[cfg(feature = "userdata-wrappers")]
        Some(type_id) if type_id == type_hints.arc_pl_rwlock => {
            let ud = get_userdata::<UserDataStorage<std::sync::Arc<parking_lot::RwLock<T>>>>(state, idx);
            (*ud).try_borrow_scoped_mut(|ud| {
                let mut ud = ud.try_write().ok_or(Error::UserDataBorrowMutError)?;
                Ok(f(&mut ud))
            })?
        }
        _ => Err(Error::UserDataTypeMismatch),
    }
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
    let code = cr#"
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
    "#;
    protect_lua!(state, 0, 1, |state| {
        let ret = ffi::luaL_loadbuffer(state, code.as_ptr(), code.count_bytes(), cstr!("=__mlua_index"));
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
    let code = cr#"
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
    "#;
    protect_lua!(state, 0, 1, |state| {
        let code_len = code.count_bytes();
        let ret = ffi::luaL_loadbuffer(state, code.as_ptr(), code_len, cstr!("=__mlua_newindex"));
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

// This method is called by Lua GC when it's time to collect the userdata.
//
// This method is usually used to collect internal userdata.
#[cfg(not(feature = "luau"))]
pub(crate) unsafe extern "C-unwind" fn collect_userdata<T>(state: *mut ffi::lua_State) -> c_int {
    let ud = get_userdata::<T>(state, -1);
    ptr::drop_in_place(ud);
    0
}

// This method is called by Luau GC when it's time to collect the userdata.
#[cfg(feature = "luau")]
pub(crate) unsafe extern "C" fn collect_userdata<T>(
    state: *mut ffi::lua_State,
    ud: *mut std::os::raw::c_void,
) {
    // Almost none Lua operations are allowed when destructor is running,
    // so we need to set a flag to prevent calling any Lua functions
    let extra = (*ffi::lua_callbacks(state)).userdata as *mut crate::state::ExtraData;
    (*extra).running_gc = true;
    // Luau does not support _any_ panics in destructors (they are declared as "C", NOT as "C-unwind"),
    // so any panics will trigger `abort()`.
    ptr::drop_in_place(ud as *mut T);
    (*extra).running_gc = false;
}

// This method can be called by user or Lua GC to destroy the userdata.
// It checks if the userdata is safe to destroy and sets the "destroyed" metatable
// to prevent further GC collection.
pub(super) unsafe extern "C-unwind" fn destroy_userdata_storage<T>(state: *mut ffi::lua_State) -> c_int {
    let ud = get_userdata::<UserDataStorage<T>>(state, 1);
    if (*ud).is_safe_to_destroy() {
        take_userdata::<UserDataStorage<T>>(state, 1);
        ffi::lua_pushboolean(state, 1);
    } else {
        ffi::lua_pushboolean(state, 0);
    }
    1
}

static USERDATA_METATABLE_INDEX: u8 = 0;
static USERDATA_METATABLE_NEWINDEX: u8 = 0;
