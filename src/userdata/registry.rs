#![allow(clippy::await_holding_refcell_ref, clippy::await_holding_lock)]

use std::any::TypeId;
use std::cell::RefCell;
use std::marker::PhantomData;
use std::os::raw::c_int;
use std::string::String as StdString;

use crate::error::{Error, Result};
use crate::state::Lua;
use crate::types::{Callback, MaybeSend};
use crate::userdata::{AnyUserData, MetaMethod, UserData, UserDataFields, UserDataMethods};
use crate::util::{get_userdata, short_type_name};
use crate::value::{FromLua, FromLuaMulti, IntoLua, IntoLuaMulti, Value};

use super::cell::{UserDataBorrowMut, UserDataBorrowRef, UserDataVariant};

#[cfg(feature = "async")]
use {
    crate::types::AsyncCallback,
    std::future::{self, Future},
};

/// Handle to registry for userdata methods and metamethods.
pub struct UserDataRegistry<'a, T: 'static> {
    // Fields
    pub(crate) fields: Vec<(String, Callback<'a>)>,
    pub(crate) field_getters: Vec<(String, Callback<'a>)>,
    pub(crate) field_setters: Vec<(String, Callback<'a>)>,
    pub(crate) meta_fields: Vec<(String, Callback<'a>)>,

    // Methods
    pub(crate) methods: Vec<(String, Callback<'a>)>,
    #[cfg(feature = "async")]
    pub(crate) async_methods: Vec<(String, AsyncCallback<'a>)>,
    pub(crate) meta_methods: Vec<(String, Callback<'a>)>,
    #[cfg(feature = "async")]
    pub(crate) async_meta_methods: Vec<(String, AsyncCallback<'a>)>,

    _type: PhantomData<T>,
}

impl<'a, T: 'static> UserDataRegistry<'a, T> {
    pub(crate) const fn new() -> Self {
        UserDataRegistry {
            fields: Vec::new(),
            field_getters: Vec::new(),
            field_setters: Vec::new(),
            meta_fields: Vec::new(),
            methods: Vec::new(),
            #[cfg(feature = "async")]
            async_methods: Vec::new(),
            meta_methods: Vec::new(),
            #[cfg(feature = "async")]
            async_meta_methods: Vec::new(),
            _type: PhantomData,
        }
    }

    fn box_method<M, A, R>(name: &str, method: M) -> Callback<'a>
    where
        M: Fn(&'a Lua, &T, A) -> Result<R> + MaybeSend + 'static,
        A: FromLuaMulti,
        R: IntoLuaMulti,
    {
        let name = get_function_name::<T>(name);
        macro_rules! try_self_arg {
            ($res:expr) => {
                $res.map_err(|err| Error::bad_self_argument(&name, err))?
            };
        }

        Box::new(move |rawlua, nargs| unsafe {
            if nargs == 0 {
                let err = Error::from_lua_conversion("missing argument", "userdata", None);
                try_self_arg!(Err(err));
            }
            let state = rawlua.state();
            // Find absolute "self" index before processing args
            let index = ffi::lua_absindex(state, -nargs);
            // Self was at position 1, so we pass 2 here
            let args = A::from_stack_args(nargs - 1, 2, Some(&name), rawlua);

            match try_self_arg!(rawlua.get_userdata_type_id(index)) {
                Some(id) if id == TypeId::of::<T>() => {
                    let ud = try_self_arg!(borrow_userdata_ref::<T>(state, index));
                    method(rawlua.lua(), &ud, args?)?.push_into_stack_multi(rawlua)
                }
                _ => Err(Error::bad_self_argument(&name, Error::UserDataTypeMismatch)),
            }
        })
    }

    fn box_method_mut<M, A, R>(name: &str, method: M) -> Callback<'a>
    where
        M: FnMut(&'a Lua, &mut T, A) -> Result<R> + MaybeSend + 'static,
        A: FromLuaMulti,
        R: IntoLuaMulti,
    {
        let name = get_function_name::<T>(name);
        macro_rules! try_self_arg {
            ($res:expr) => {
                $res.map_err(|err| Error::bad_self_argument(&name, err))?
            };
        }

        let method = RefCell::new(method);
        Box::new(move |rawlua, nargs| unsafe {
            let mut method = method
                .try_borrow_mut()
                .map_err(|_| Error::RecursiveMutCallback)?;
            if nargs == 0 {
                let err = Error::from_lua_conversion("missing argument", "userdata", None);
                try_self_arg!(Err(err));
            }
            let state = rawlua.state();
            // Find absolute "self" index before processing args
            let index = ffi::lua_absindex(state, -nargs);
            // Self was at position 1, so we pass 2 here
            let args = A::from_stack_args(nargs - 1, 2, Some(&name), rawlua);

            match try_self_arg!(rawlua.get_userdata_type_id(index)) {
                Some(id) if id == TypeId::of::<T>() => {
                    let mut ud = try_self_arg!(borrow_userdata_mut::<T>(state, index));
                    method(rawlua.lua(), &mut ud, args?)?.push_into_stack_multi(rawlua)
                }
                _ => Err(Error::bad_self_argument(&name, Error::UserDataTypeMismatch)),
            }
        })
    }

    #[cfg(feature = "async")]
    fn box_async_method<M, A, MR, R>(name: &str, method: M) -> AsyncCallback<'a>
    where
        M: Fn(&'a Lua, &'a T, A) -> MR + MaybeSend + 'static,
        A: FromLuaMulti,
        MR: Future<Output = Result<R>> + 'a,
        R: IntoLuaMulti,
    {
        let name = get_function_name::<T>(name);
        macro_rules! try_self_arg {
            ($res:expr) => {
                match $res {
                    Ok(res) => res,
                    Err(err) => {
                        return Box::pin(future::ready(Err(Error::bad_self_argument(&name, err))))
                    }
                }
            };
        }

        Box::new(move |rawlua, mut args| unsafe {
            let this = args
                .pop_front()
                .ok_or_else(|| Error::from_lua_conversion("missing argument", "userdata", None));
            let lua = rawlua.lua();
            let this = try_self_arg!(AnyUserData::from_lua(try_self_arg!(this), lua));
            let args = A::from_lua_args(args, 2, Some(&name), lua);

            let (ref_thread, index) = (rawlua.ref_thread(), this.0.index);
            match try_self_arg!(this.type_id()) {
                Some(id) if id == TypeId::of::<T>() => {
                    let ud = try_self_arg!(borrow_userdata_ref::<T>(ref_thread, index));
                    let args = match args {
                        Ok(args) => args,
                        Err(e) => return Box::pin(future::ready(Err(e))),
                    };
                    let fut = method(lua, ud.get_ref(), args);
                    Box::pin(async move { fut.await?.push_into_stack_multi(rawlua) })
                }
                _ => {
                    let err = Error::bad_self_argument(&name, Error::UserDataTypeMismatch);
                    Box::pin(future::ready(Err(err)))
                }
            }
        })
    }

    #[cfg(feature = "async")]
    fn box_async_method_mut<M, A, MR, R>(name: &str, method: M) -> AsyncCallback<'a>
    where
        M: Fn(&'a Lua, &'a mut T, A) -> MR + MaybeSend + 'static,
        A: FromLuaMulti,
        MR: Future<Output = Result<R>> + 'a,
        R: IntoLuaMulti,
    {
        let name = get_function_name::<T>(name);
        macro_rules! try_self_arg {
            ($res:expr) => {
                match $res {
                    Ok(res) => res,
                    Err(err) => {
                        return Box::pin(future::ready(Err(Error::bad_self_argument(&name, err))))
                    }
                }
            };
        }

        Box::new(move |rawlua, mut args| unsafe {
            let this = args
                .pop_front()
                .ok_or_else(|| Error::from_lua_conversion("missing argument", "userdata", None));
            let lua = rawlua.lua();
            let this = try_self_arg!(AnyUserData::from_lua(try_self_arg!(this), lua));
            let args = A::from_lua_args(args, 2, Some(&name), lua);

            let (ref_thread, index) = (rawlua.ref_thread(), this.0.index);
            match try_self_arg!(this.type_id()) {
                Some(id) if id == TypeId::of::<T>() => {
                    let mut ud = try_self_arg!(borrow_userdata_mut::<T>(ref_thread, index));
                    let args = match args {
                        Ok(args) => args,
                        Err(e) => return Box::pin(future::ready(Err(e))),
                    };
                    let fut = method(lua, ud.get_mut(), args);
                    Box::pin(async move { fut.await?.push_into_stack_multi(rawlua) })
                }
                _ => {
                    let err = Error::bad_self_argument(&name, Error::UserDataTypeMismatch);
                    Box::pin(future::ready(Err(err)))
                }
            }
        })
    }

    fn box_function<F, A, R>(name: &str, function: F) -> Callback<'a>
    where
        F: Fn(&'a Lua, A) -> Result<R> + MaybeSend + 'static,
        A: FromLuaMulti,
        R: IntoLuaMulti,
    {
        let name = get_function_name::<T>(name);
        Box::new(move |lua, nargs| unsafe {
            let args = A::from_stack_args(nargs, 1, Some(&name), lua)?;
            function(lua.lua(), args)?.push_into_stack_multi(lua)
        })
    }

    fn box_function_mut<F, A, R>(name: &str, function: F) -> Callback<'a>
    where
        F: FnMut(&'a Lua, A) -> Result<R> + MaybeSend + 'static,
        A: FromLuaMulti,
        R: IntoLuaMulti,
    {
        let name = get_function_name::<T>(name);
        let function = RefCell::new(function);
        Box::new(move |lua, nargs| unsafe {
            let function = &mut *function
                .try_borrow_mut()
                .map_err(|_| Error::RecursiveMutCallback)?;
            let args = A::from_stack_args(nargs, 1, Some(&name), lua)?;
            function(lua.lua(), args)?.push_into_stack_multi(lua)
        })
    }

    #[cfg(feature = "async")]
    fn box_async_function<F, A, FR, R>(name: &str, function: F) -> AsyncCallback<'a>
    where
        F: Fn(&'a Lua, A) -> FR + MaybeSend + 'static,
        A: FromLuaMulti,
        FR: Future<Output = Result<R>> + 'a,
        R: IntoLuaMulti,
    {
        let name = get_function_name::<T>(name);
        Box::new(move |rawlua, args| unsafe {
            let lua = rawlua.lua();
            let args = match A::from_lua_args(args, 1, Some(&name), lua) {
                Ok(args) => args,
                Err(e) => return Box::pin(future::ready(Err(e))),
            };
            let fut = function(lua, args);
            Box::pin(async move { fut.await?.push_into_stack_multi(rawlua) })
        })
    }

    pub(crate) fn check_meta_field<V>(lua: &Lua, name: &str, value: V) -> Result<Value>
    where
        V: IntoLua,
    {
        let value = value.into_lua(lua)?;
        if name == MetaMethod::Index || name == MetaMethod::NewIndex {
            match value {
                Value::Nil | Value::Table(_) | Value::Function(_) => {}
                _ => {
                    return Err(Error::MetaMethodTypeError {
                        method: name.to_string(),
                        type_name: value.type_name(),
                        message: Some("expected nil, table or function".to_string()),
                    })
                }
            }
        }
        value.into_lua(lua)
    }
}

// Returns function name for the type `T`, without the module path
fn get_function_name<T>(name: &str) -> StdString {
    format!("{}.{name}", short_type_name::<T>())
}

impl<'a, T: 'static> UserDataFields<'a, T> for UserDataRegistry<'a, T> {
    fn add_field<V>(&mut self, name: impl ToString, value: V)
    where
        V: IntoLua + Clone + 'static,
    {
        let name = name.to_string();
        let callback = Box::new(move |lua, _| unsafe { value.clone().push_into_stack_multi(lua) });
        self.fields.push((name, callback));
    }

    fn add_field_method_get<M, R>(&mut self, name: impl ToString, method: M)
    where
        M: Fn(&'a Lua, &T) -> Result<R> + MaybeSend + 'static,
        R: IntoLua,
    {
        let name = name.to_string();
        let callback = Self::box_method(&name, move |lua, data, ()| method(lua, data));
        self.field_getters.push((name, callback));
    }

    fn add_field_method_set<M, A>(&mut self, name: impl ToString, method: M)
    where
        M: FnMut(&'a Lua, &mut T, A) -> Result<()> + MaybeSend + 'static,
        A: FromLua,
    {
        let name = name.to_string();
        let callback = Self::box_method_mut(&name, method);
        self.field_setters.push((name, callback));
    }

    fn add_field_function_get<F, R>(&mut self, name: impl ToString, function: F)
    where
        F: Fn(&'a Lua, AnyUserData) -> Result<R> + MaybeSend + 'static,
        R: IntoLua,
    {
        let name = name.to_string();
        let callback = Self::box_function(&name, function);
        self.field_getters.push((name, callback));
    }

    fn add_field_function_set<F, A>(&mut self, name: impl ToString, mut function: F)
    where
        F: FnMut(&'a Lua, AnyUserData, A) -> Result<()> + MaybeSend + 'static,
        A: FromLua,
    {
        let name = name.to_string();
        let callback =
            Self::box_function_mut(&name, move |lua, (data, val)| function(lua, data, val));
        self.field_setters.push((name, callback));
    }

    fn add_meta_field<V>(&mut self, name: impl ToString, value: V)
    where
        V: IntoLua + Clone + 'static,
    {
        let name = name.to_string();
        self.meta_fields.push((
            name.clone(),
            Box::new(move |lua, _| unsafe {
                Self::check_meta_field(lua.lua(), &name, value.clone())?.push_into_stack_multi(lua)
            }),
        ));
    }

    fn add_meta_field_with<F, R>(&mut self, name: impl ToString, f: F)
    where
        F: Fn(&'a Lua) -> Result<R> + MaybeSend + 'static,
        R: IntoLua,
    {
        let name = name.to_string();
        self.meta_fields.push((
            name.clone(),
            Box::new(move |rawlua, _| unsafe {
                let lua = rawlua.lua();
                Self::check_meta_field(lua, &name, f(lua)?)?.push_into_stack_multi(rawlua)
            }),
        ));
    }
}

impl<'a, T: 'static> UserDataMethods<'a, T> for UserDataRegistry<'a, T> {
    fn add_method<M, A, R>(&mut self, name: impl ToString, method: M)
    where
        M: Fn(&'a Lua, &T, A) -> Result<R> + MaybeSend + 'static,
        A: FromLuaMulti,
        R: IntoLuaMulti,
    {
        let name = name.to_string();
        let callback = Self::box_method(&name, method);
        self.methods.push((name, callback));
    }

    fn add_method_mut<M, A, R>(&mut self, name: impl ToString, method: M)
    where
        M: FnMut(&'a Lua, &mut T, A) -> Result<R> + MaybeSend + 'static,
        A: FromLuaMulti,
        R: IntoLuaMulti,
    {
        let name = name.to_string();
        let callback = Self::box_method_mut(&name, method);
        self.methods.push((name, callback));
    }

    #[cfg(feature = "async")]
    fn add_async_method<M, A, MR, R>(&mut self, name: impl ToString, method: M)
    where
        M: Fn(&'a Lua, &'a T, A) -> MR + MaybeSend + 'static,
        A: FromLuaMulti,
        MR: Future<Output = Result<R>> + 'a,
        R: IntoLuaMulti,
    {
        let name = name.to_string();
        let callback = Self::box_async_method(&name, method);
        self.async_methods.push((name, callback));
    }

    #[cfg(feature = "async")]
    fn add_async_method_mut<M, A, MR, R>(&mut self, name: impl ToString, method: M)
    where
        M: Fn(&'a Lua, &'a mut T, A) -> MR + MaybeSend + 'static,
        A: FromLuaMulti,
        MR: Future<Output = Result<R>> + 'a,
        R: IntoLuaMulti,
    {
        let name = name.to_string();
        let callback = Self::box_async_method_mut(&name, method);
        self.async_methods.push((name, callback));
    }

    fn add_function<F, A, R>(&mut self, name: impl ToString, function: F)
    where
        F: Fn(&'a Lua, A) -> Result<R> + MaybeSend + 'static,
        A: FromLuaMulti,
        R: IntoLuaMulti,
    {
        let name = name.to_string();
        let callback = Self::box_function(&name, function);
        self.methods.push((name, callback));
    }

    fn add_function_mut<F, A, R>(&mut self, name: impl ToString, function: F)
    where
        F: FnMut(&'a Lua, A) -> Result<R> + MaybeSend + 'static,
        A: FromLuaMulti,
        R: IntoLuaMulti,
    {
        let name = name.to_string();
        let callback = Self::box_function_mut(&name, function);
        self.methods.push((name, callback));
    }

    #[cfg(feature = "async")]
    fn add_async_function<F, A, FR, R>(&mut self, name: impl ToString, function: F)
    where
        F: Fn(&'a Lua, A) -> FR + MaybeSend + 'static,
        A: FromLuaMulti,
        FR: Future<Output = Result<R>> + 'a,
        R: IntoLuaMulti,
    {
        let name = name.to_string();
        let callback = Self::box_async_function(&name, function);
        self.async_methods.push((name, callback));
    }

    fn add_meta_method<M, A, R>(&mut self, name: impl ToString, method: M)
    where
        M: Fn(&'a Lua, &T, A) -> Result<R> + MaybeSend + 'static,
        A: FromLuaMulti,
        R: IntoLuaMulti,
    {
        let name = name.to_string();
        let callback = Self::box_method(&name, method);
        self.meta_methods.push((name, callback));
    }

    fn add_meta_method_mut<M, A, R>(&mut self, name: impl ToString, method: M)
    where
        M: FnMut(&'a Lua, &mut T, A) -> Result<R> + MaybeSend + 'static,
        A: FromLuaMulti,
        R: IntoLuaMulti,
    {
        let name = name.to_string();
        let callback = Self::box_method_mut(&name, method);
        self.meta_methods.push((name, callback));
    }

    #[cfg(all(feature = "async", not(any(feature = "lua51", feature = "luau"))))]
    fn add_async_meta_method<M, A, MR, R>(&mut self, name: impl ToString, method: M)
    where
        M: Fn(&'a Lua, &'a T, A) -> MR + MaybeSend + 'static,
        A: FromLuaMulti,
        MR: Future<Output = Result<R>> + 'a,
        R: IntoLuaMulti,
    {
        let name = name.to_string();
        let callback = Self::box_async_method(&name, method);
        self.async_meta_methods.push((name, callback));
    }

    #[cfg(all(feature = "async", not(any(feature = "lua51", feature = "luau"))))]
    fn add_async_meta_method_mut<M, A, MR, R>(&mut self, name: impl ToString, method: M)
    where
        M: Fn(&'a Lua, &'a mut T, A) -> MR + MaybeSend + 'static,
        A: FromLuaMulti,
        MR: Future<Output = Result<R>> + 'a,
        R: IntoLuaMulti,
    {
        let name = name.to_string();
        let callback = Self::box_async_method_mut(&name, method);
        self.async_meta_methods.push((name, callback));
    }

    fn add_meta_function<F, A, R>(&mut self, name: impl ToString, function: F)
    where
        F: Fn(&'a Lua, A) -> Result<R> + MaybeSend + 'static,
        A: FromLuaMulti,
        R: IntoLuaMulti,
    {
        let name = name.to_string();
        let callback = Self::box_function(&name, function);
        self.meta_methods.push((name, callback));
    }

    fn add_meta_function_mut<F, A, R>(&mut self, name: impl ToString, function: F)
    where
        F: FnMut(&'a Lua, A) -> Result<R> + MaybeSend + 'static,
        A: FromLuaMulti,
        R: IntoLuaMulti,
    {
        let name = name.to_string();
        let callback = Self::box_function_mut(&name, function);
        self.meta_methods.push((name, callback));
    }

    #[cfg(all(feature = "async", not(any(feature = "lua51", feature = "luau"))))]
    fn add_async_meta_function<F, A, FR, R>(&mut self, name: impl ToString, function: F)
    where
        F: Fn(&'a Lua, A) -> FR + MaybeSend + 'static,
        A: FromLuaMulti,
        FR: Future<Output = Result<R>> + 'a,
        R: IntoLuaMulti,
    {
        let name = name.to_string();
        let callback = Self::box_async_function(&name, function);
        self.async_meta_methods.push((name, callback));
    }
}

// Borrow the userdata in-place from the Lua stack
#[inline(always)]
unsafe fn borrow_userdata_ref<'a, T>(
    state: *mut ffi::lua_State,
    index: c_int,
) -> Result<UserDataBorrowRef<'a, T>> {
    let ud = get_userdata::<UserDataVariant<T>>(state, index);
    (*ud).try_borrow()
}

// Borrow the userdata mutably in-place from the Lua stack
#[inline(always)]
unsafe fn borrow_userdata_mut<'a, T>(
    state: *mut ffi::lua_State,
    index: c_int,
) -> Result<UserDataBorrowMut<'a, T>> {
    let ud = get_userdata::<UserDataVariant<T>>(state, index);
    (*ud).try_borrow_mut()
}

macro_rules! lua_userdata_impl {
    ($type:ty) => {
        impl<T: UserData + 'static> UserData for $type {
            fn register(registry: &mut UserDataRegistry<Self>) {
                let mut orig_registry = UserDataRegistry::new();
                T::register(&mut orig_registry);

                // Copy all fields, methods, etc. from the original registry
                registry.fields.extend(orig_registry.fields);
                registry.field_getters.extend(orig_registry.field_getters);
                registry.field_setters.extend(orig_registry.field_setters);
                registry.meta_fields.extend(orig_registry.meta_fields);
                registry.methods.extend(orig_registry.methods);
                #[cfg(feature = "async")]
                registry.async_methods.extend(orig_registry.async_methods);
                registry.meta_methods.extend(orig_registry.meta_methods);
                #[cfg(feature = "async")]
                registry
                    .async_meta_methods
                    .extend(orig_registry.async_meta_methods);
            }
        }
    };
}

// A special proxy object for UserData
pub(crate) struct UserDataProxy<T>(pub(crate) PhantomData<T>);

lua_userdata_impl!(UserDataProxy<T>);
