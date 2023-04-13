use std::any::{self, TypeId};
use std::cell::{Ref, RefCell, RefMut};
use std::marker::PhantomData;
use std::string::String as StdString;
use std::sync::{Arc, Mutex, RwLock};

use crate::error::{Error, Result};
use crate::lua::Lua;
use crate::types::{Callback, MaybeSend};
use crate::userdata::{
    AnyUserData, MetaMethod, UserData, UserDataCell, UserDataFields, UserDataMethods,
};
use crate::util::{check_stack, get_userdata, StackGuard};
use crate::value::{FromLua, FromLuaMulti, IntoLua, IntoLuaMulti, Value};

#[cfg(not(feature = "send"))]
use std::rc::Rc;

#[cfg(feature = "async")]
use {
    crate::types::AsyncCallback,
    futures_util::future::{self, TryFutureExt},
    std::future::Future,
};

pub struct UserDataRegistrar<'lua, T: 'static> {
    // Fields
    pub(crate) field_getters: Vec<(String, Callback<'lua, 'static>)>,
    pub(crate) field_setters: Vec<(String, Callback<'lua, 'static>)>,
    #[allow(clippy::type_complexity)]
    pub(crate) meta_fields: Vec<(
        String,
        Box<dyn Fn(&'lua Lua) -> Result<Value<'lua>> + 'static>,
    )>,

    // Methods
    pub(crate) methods: Vec<(String, Callback<'lua, 'static>)>,
    #[cfg(feature = "async")]
    pub(crate) async_methods: Vec<(String, AsyncCallback<'lua, 'static>)>,
    pub(crate) meta_methods: Vec<(String, Callback<'lua, 'static>)>,
    #[cfg(feature = "async")]
    pub(crate) async_meta_methods: Vec<(String, AsyncCallback<'lua, 'static>)>,

    _type: PhantomData<T>,
}

impl<'lua, T: 'static> UserDataRegistrar<'lua, T> {
    pub(crate) const fn new() -> Self {
        UserDataRegistrar {
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

    fn box_method<M, A, R>(name: &str, method: M) -> Callback<'lua, 'static>
    where
        M: Fn(&'lua Lua, &T, A) -> Result<R> + MaybeSend + 'static,
        A: FromLuaMulti<'lua>,
        R: IntoLuaMulti<'lua>,
    {
        let name = get_function_name::<T>(name);
        macro_rules! try_self_arg {
            ($res:expr) => {
                $res.map_err(|err| Error::bad_self_argument(&name, err))?
            };
            ($res:expr, $err:expr) => {
                $res.map_err(|_| Error::bad_self_argument(&name, $err))?
            };
        }

        Box::new(move |lua, mut args| {
            let front = args.pop_front();
            let call = |ud| {
                // Self was at index 1, so we pass 2 here
                let args = A::from_lua_multi_args(args, 2, Some(&name), lua)?;
                method(lua, ud, args)?.into_lua_multi(lua)
            };

            if let Some(front) = front {
                let state = lua.state();
                let userdata = try_self_arg!(AnyUserData::from_lua(front, lua));
                unsafe {
                    let _sg = StackGuard::new(state);
                    check_stack(state, 2)?;

                    let type_id = try_self_arg!(lua.push_userdata_ref(&userdata.0));
                    match type_id {
                        Some(id) if id == TypeId::of::<T>() => {
                            let ud = try_self_arg!(get_userdata_ref::<T>(state));
                            call(&ud)
                        }
                        #[cfg(not(feature = "send"))]
                        Some(id) if id == TypeId::of::<Rc<RefCell<T>>>() => {
                            let ud = try_self_arg!(get_userdata_ref::<Rc<RefCell<T>>>(state));
                            let ud = try_self_arg!(ud.try_borrow(), Error::UserDataBorrowError);
                            call(&ud)
                        }
                        Some(id) if id == TypeId::of::<Arc<Mutex<T>>>() => {
                            let ud = try_self_arg!(get_userdata_ref::<Arc<Mutex<T>>>(state));
                            let ud = try_self_arg!(ud.try_lock(), Error::UserDataBorrowError);
                            call(&ud)
                        }
                        #[cfg(feature = "parking_lot")]
                        Some(id) if id == TypeId::of::<Arc<parking_lot::Mutex<T>>>() => {
                            let ud = get_userdata_ref::<Arc<parking_lot::Mutex<T>>>(state);
                            let ud = try_self_arg!(ud);
                            let ud = try_self_arg!(ud.try_lock().ok_or(Error::UserDataBorrowError));
                            call(&ud)
                        }
                        Some(id) if id == TypeId::of::<Arc<RwLock<T>>>() => {
                            let ud = try_self_arg!(get_userdata_ref::<Arc<RwLock<T>>>(state));
                            let ud = try_self_arg!(ud.try_read(), Error::UserDataBorrowError);
                            call(&ud)
                        }
                        #[cfg(feature = "parking_lot")]
                        Some(id) if id == TypeId::of::<Arc<parking_lot::RwLock<T>>>() => {
                            let ud = get_userdata_ref::<Arc<parking_lot::RwLock<T>>>(state);
                            let ud = try_self_arg!(ud);
                            let ud = try_self_arg!(ud.try_read().ok_or(Error::UserDataBorrowError));
                            call(&ud)
                        }
                        _ => Err(Error::bad_self_argument(&name, Error::UserDataTypeMismatch)),
                    }
                }
            } else {
                let err = Error::from_lua_conversion("missing argument", "userdata", None);
                Err(Error::bad_self_argument(&name, err))
            }
        })
    }

    fn box_method_mut<M, A, R>(name: &str, method: M) -> Callback<'lua, 'static>
    where
        M: FnMut(&'lua Lua, &mut T, A) -> Result<R> + MaybeSend + 'static,
        A: FromLuaMulti<'lua>,
        R: IntoLuaMulti<'lua>,
    {
        let name = get_function_name::<T>(name);
        macro_rules! try_self_arg {
            ($res:expr) => {
                $res.map_err(|err| Error::bad_self_argument(&name, err))?
            };
            ($res:expr, $err:expr) => {
                $res.map_err(|_| Error::bad_self_argument(&name, $err))?
            };
        }

        let method = RefCell::new(method);
        Box::new(move |lua, mut args| {
            let mut method = method
                .try_borrow_mut()
                .map_err(|_| Error::RecursiveMutCallback)?;
            let front = args.pop_front();
            let call = |ud| {
                // Self was at index 1, so we pass 2 here
                let args = A::from_lua_multi_args(args, 2, Some(&name), lua)?;
                method(lua, ud, args)?.into_lua_multi(lua)
            };

            if let Some(front) = front {
                let state = lua.state();
                let userdata = try_self_arg!(AnyUserData::from_lua(front, lua));
                unsafe {
                    let _sg = StackGuard::new(state);
                    check_stack(state, 2)?;

                    let type_id = try_self_arg!(lua.push_userdata_ref(&userdata.0));
                    match type_id {
                        Some(id) if id == TypeId::of::<T>() => {
                            let mut ud = try_self_arg!(get_userdata_mut::<T>(state));
                            call(&mut ud)
                        }
                        #[cfg(not(feature = "send"))]
                        Some(id) if id == TypeId::of::<Rc<RefCell<T>>>() => {
                            let ud = try_self_arg!(get_userdata_mut::<Rc<RefCell<T>>>(state));
                            let mut ud =
                                try_self_arg!(ud.try_borrow_mut(), Error::UserDataBorrowMutError);
                            call(&mut ud)
                        }
                        Some(id) if id == TypeId::of::<Arc<Mutex<T>>>() => {
                            let ud = try_self_arg!(get_userdata_mut::<Arc<Mutex<T>>>(state));
                            let mut ud =
                                try_self_arg!(ud.try_lock(), Error::UserDataBorrowMutError);
                            call(&mut ud)
                        }
                        #[cfg(feature = "parking_lot")]
                        Some(id) if id == TypeId::of::<Arc<parking_lot::Mutex<T>>>() => {
                            let ud = get_userdata_mut::<Arc<parking_lot::Mutex<T>>>(state);
                            let ud = try_self_arg!(ud);
                            let mut ud =
                                try_self_arg!(ud.try_lock().ok_or(Error::UserDataBorrowMutError));
                            call(&mut ud)
                        }
                        Some(id) if id == TypeId::of::<Arc<RwLock<T>>>() => {
                            let ud = try_self_arg!(get_userdata_mut::<Arc<RwLock<T>>>(state));
                            let mut ud =
                                try_self_arg!(ud.try_write(), Error::UserDataBorrowMutError);
                            call(&mut ud)
                        }
                        #[cfg(feature = "parking_lot")]
                        Some(id) if id == TypeId::of::<Arc<parking_lot::RwLock<T>>>() => {
                            let ud = get_userdata_mut::<Arc<parking_lot::RwLock<T>>>(state);
                            let ud = try_self_arg!(ud);
                            let mut ud =
                                try_self_arg!(ud.try_write().ok_or(Error::UserDataBorrowMutError));
                            call(&mut ud)
                        }
                        _ => Err(Error::bad_self_argument(&name, Error::UserDataTypeMismatch)),
                    }
                }
            } else {
                let err = Error::from_lua_conversion("missing argument", "userdata", None);
                Err(Error::bad_self_argument(&name, err))
            }
        })
    }

    #[cfg(feature = "async")]
    fn box_async_method<M, A, MR, R>(name: &str, method: M) -> AsyncCallback<'lua, 'static>
    where
        T: Clone,
        M: Fn(&'lua Lua, T, A) -> MR + MaybeSend + 'static,
        A: FromLuaMulti<'lua>,
        MR: Future<Output = Result<R>> + 'lua,
        R: IntoLuaMulti<'lua>,
    {
        let name = get_function_name::<T>(name);
        macro_rules! try_self_arg {
            ($res:expr) => {
                $res.map_err(|err| Error::bad_self_argument(&name, err))?
            };
            ($res:expr, $err:expr) => {
                $res.map_err(|_| Error::bad_self_argument(&name, $err))?
            };
        }

        Box::new(move |lua, mut args| {
            let front = args.pop_front();
            let call = |ud| {
                // Self was at index 1, so we pass 2 here
                let args = A::from_lua_multi_args(args, 2, Some(&name), lua)?;
                Ok(method(lua, ud, args))
            };

            let fut_res = || {
                if let Some(front) = front {
                    let state = lua.state();
                    let userdata = AnyUserData::from_lua(front, lua)?;
                    unsafe {
                        let _sg = StackGuard::new(state);
                        check_stack(state, 2)?;

                        let type_id = try_self_arg!(lua.push_userdata_ref(&userdata.0));
                        match type_id {
                            Some(id) if id == TypeId::of::<T>() => {
                                let ud = get_userdata_ref::<T>(state)?;
                                call(ud.clone())
                            }
                            #[cfg(not(feature = "send"))]
                            Some(id) if id == TypeId::of::<Rc<RefCell<T>>>() => {
                                let ud = try_self_arg!(get_userdata_ref::<Rc<RefCell<T>>>(state));
                                let ud = try_self_arg!(ud.try_borrow(), Error::UserDataBorrowError);
                                call(ud.clone())
                            }
                            Some(id) if id == TypeId::of::<Arc<Mutex<T>>>() => {
                                let ud = try_self_arg!(get_userdata_ref::<Arc<Mutex<T>>>(state));
                                let ud = try_self_arg!(ud.try_lock(), Error::UserDataBorrowError);
                                call(ud.clone())
                            }
                            #[cfg(feature = "parking_lot")]
                            Some(id) if id == TypeId::of::<Arc<parking_lot::Mutex<T>>>() => {
                                let ud = get_userdata_ref::<Arc<parking_lot::Mutex<T>>>(state);
                                let ud = try_self_arg!(ud);
                                let ud =
                                    try_self_arg!(ud.try_lock().ok_or(Error::UserDataBorrowError));
                                call(ud.clone())
                            }
                            Some(id) if id == TypeId::of::<Arc<RwLock<T>>>() => {
                                let ud = try_self_arg!(get_userdata_ref::<Arc<RwLock<T>>>(state));
                                let ud = try_self_arg!(ud.try_read(), Error::UserDataBorrowError);
                                call(ud.clone())
                            }
                            #[cfg(feature = "parking_lot")]
                            Some(id) if id == TypeId::of::<Arc<parking_lot::RwLock<T>>>() => {
                                let ud = get_userdata_ref::<Arc<parking_lot::RwLock<T>>>(state);
                                let ud = try_self_arg!(ud);
                                let ud =
                                    try_self_arg!(ud.try_read().ok_or(Error::UserDataBorrowError));
                                call(ud.clone())
                            }
                            _ => Err(Error::bad_self_argument(&name, Error::UserDataTypeMismatch)),
                        }
                    }
                } else {
                    let err = Error::from_lua_conversion("missing argument", "userdata", None);
                    Err(Error::bad_self_argument(&name, err))
                }
            };
            match fut_res() {
                Ok(fut) => {
                    Box::pin(fut.and_then(move |ret| future::ready(ret.into_lua_multi(lua))))
                }
                Err(e) => Box::pin(future::err(e)),
            }
        })
    }

    fn box_function<F, A, R>(name: &str, function: F) -> Callback<'lua, 'static>
    where
        F: Fn(&'lua Lua, A) -> Result<R> + MaybeSend + 'static,
        A: FromLuaMulti<'lua>,
        R: IntoLuaMulti<'lua>,
    {
        let name = get_function_name::<T>(name);
        Box::new(move |lua, args| {
            function(lua, A::from_lua_multi_args(args, 1, Some(&name), lua)?)?.into_lua_multi(lua)
        })
    }

    fn box_function_mut<F, A, R>(name: &str, function: F) -> Callback<'lua, 'static>
    where
        F: FnMut(&'lua Lua, A) -> Result<R> + MaybeSend + 'static,
        A: FromLuaMulti<'lua>,
        R: IntoLuaMulti<'lua>,
    {
        let name = get_function_name::<T>(name);
        let function = RefCell::new(function);
        Box::new(move |lua, args| {
            let function = &mut *function
                .try_borrow_mut()
                .map_err(|_| Error::RecursiveMutCallback)?;
            function(lua, A::from_lua_multi_args(args, 1, Some(&name), lua)?)?.into_lua_multi(lua)
        })
    }

    #[cfg(feature = "async")]
    fn box_async_function<F, A, FR, R>(name: &str, function: F) -> AsyncCallback<'lua, 'static>
    where
        F: Fn(&'lua Lua, A) -> FR + MaybeSend + 'static,
        A: FromLuaMulti<'lua>,
        FR: Future<Output = Result<R>> + 'lua,
        R: IntoLuaMulti<'lua>,
    {
        let name = get_function_name::<T>(name);
        Box::new(move |lua, args| {
            let args = match A::from_lua_multi_args(args, 1, Some(&name), lua) {
                Ok(args) => args,
                Err(e) => return Box::pin(future::err(e)),
            };
            Box::pin(
                function(lua, args).and_then(move |ret| future::ready(ret.into_lua_multi(lua))),
            )
        })
    }
}

// Returns function name for the type `T`, without the module path
fn get_function_name<T: 'static>(name: &str) -> StdString {
    let type_name = any::type_name::<T>().rsplit("::").next().unwrap();
    format!("{type_name}.{name}",)
}

impl<'lua, T: 'static> UserDataFields<'lua, T> for UserDataRegistrar<'lua, T> {
    fn add_field_method_get<M, R>(&mut self, name: impl AsRef<str>, method: M)
    where
        M: Fn(&'lua Lua, &T) -> Result<R> + MaybeSend + 'static,
        R: IntoLua<'lua>,
    {
        let name = name.as_ref();
        let method = Self::box_method(name, move |lua, data, ()| method(lua, data));
        self.field_getters.push((name.into(), method));
    }

    fn add_field_method_set<M, A>(&mut self, name: impl AsRef<str>, method: M)
    where
        M: FnMut(&'lua Lua, &mut T, A) -> Result<()> + MaybeSend + 'static,
        A: FromLua<'lua>,
    {
        let name = name.as_ref();
        let method = Self::box_method_mut(name, method);
        self.field_setters.push((name.into(), method));
    }

    fn add_field_function_get<F, R>(&mut self, name: impl AsRef<str>, function: F)
    where
        F: Fn(&'lua Lua, AnyUserData<'lua>) -> Result<R> + MaybeSend + 'static,
        R: IntoLua<'lua>,
    {
        let name = name.as_ref();
        let func = Self::box_function(name, function);
        self.field_getters.push((name.into(), func));
    }

    fn add_field_function_set<F, A>(&mut self, name: impl AsRef<str>, mut function: F)
    where
        F: FnMut(&'lua Lua, AnyUserData<'lua>, A) -> Result<()> + MaybeSend + 'static,
        A: FromLua<'lua>,
    {
        let name = name.as_ref();
        let func = Self::box_function_mut(name, move |lua, (data, val)| function(lua, data, val));
        self.field_setters.push((name.into(), func));
    }

    fn add_meta_field_with<F, R>(&mut self, name: impl AsRef<str>, f: F)
    where
        F: Fn(&'lua Lua) -> Result<R> + MaybeSend + 'static,
        R: IntoLua<'lua>,
    {
        let name = name.as_ref().to_string();
        self.meta_fields.push((
            name.clone(),
            Box::new(move |lua| {
                let value = f(lua)?.into_lua(lua)?;
                if name == MetaMethod::Index || name == MetaMethod::NewIndex {
                    match value {
                        Value::Nil | Value::Table(_) | Value::Function(_) => {}
                        _ => {
                            return Err(Error::MetaMethodTypeError {
                                method: name.clone(),
                                type_name: value.type_name(),
                                message: Some("expected nil, table or function".to_string()),
                            })
                        }
                    }
                }
                Ok(value)
            }),
        ));
    }

    // Below are internal methods

    fn add_field_getter(&mut self, name: String, callback: Callback<'lua, 'static>) {
        self.field_getters.push((name, callback));
    }

    fn add_field_setter(&mut self, name: String, callback: Callback<'lua, 'static>) {
        self.field_setters.push((name, callback));
    }
}

impl<'lua, T: 'static> UserDataMethods<'lua, T> for UserDataRegistrar<'lua, T> {
    fn add_method<M, A, R>(&mut self, name: impl AsRef<str>, method: M)
    where
        M: Fn(&'lua Lua, &T, A) -> Result<R> + MaybeSend + 'static,
        A: FromLuaMulti<'lua>,
        R: IntoLuaMulti<'lua>,
    {
        let name = name.as_ref();
        self.methods
            .push((name.into(), Self::box_method(name, method)));
    }

    fn add_method_mut<M, A, R>(&mut self, name: impl AsRef<str>, method: M)
    where
        M: FnMut(&'lua Lua, &mut T, A) -> Result<R> + MaybeSend + 'static,
        A: FromLuaMulti<'lua>,
        R: IntoLuaMulti<'lua>,
    {
        let name = name.as_ref();
        self.methods
            .push((name.into(), Self::box_method_mut(name, method)));
    }

    #[cfg(feature = "async")]
    fn add_async_method<M, A, MR, R>(&mut self, name: impl AsRef<str>, method: M)
    where
        T: Clone,
        M: Fn(&'lua Lua, T, A) -> MR + MaybeSend + 'static,
        A: FromLuaMulti<'lua>,
        MR: Future<Output = Result<R>> + 'lua,
        R: IntoLuaMulti<'lua>,
    {
        let name = name.as_ref();
        self.async_methods
            .push((name.into(), Self::box_async_method(name, method)));
    }

    fn add_function<F, A, R>(&mut self, name: impl AsRef<str>, function: F)
    where
        F: Fn(&'lua Lua, A) -> Result<R> + MaybeSend + 'static,
        A: FromLuaMulti<'lua>,
        R: IntoLuaMulti<'lua>,
    {
        let name = name.as_ref();
        self.methods
            .push((name.into(), Self::box_function(name, function)));
    }

    fn add_function_mut<F, A, R>(&mut self, name: impl AsRef<str>, function: F)
    where
        F: FnMut(&'lua Lua, A) -> Result<R> + MaybeSend + 'static,
        A: FromLuaMulti<'lua>,
        R: IntoLuaMulti<'lua>,
    {
        let name = name.as_ref();
        self.methods
            .push((name.into(), Self::box_function_mut(name, function)));
    }

    #[cfg(feature = "async")]
    fn add_async_function<F, A, FR, R>(&mut self, name: impl AsRef<str>, function: F)
    where
        F: Fn(&'lua Lua, A) -> FR + MaybeSend + 'static,
        A: FromLuaMulti<'lua>,
        FR: Future<Output = Result<R>> + 'lua,
        R: IntoLuaMulti<'lua>,
    {
        let name = name.as_ref();
        self.async_methods
            .push((name.into(), Self::box_async_function(name, function)));
    }

    fn add_meta_method<M, A, R>(&mut self, name: impl AsRef<str>, method: M)
    where
        M: Fn(&'lua Lua, &T, A) -> Result<R> + MaybeSend + 'static,
        A: FromLuaMulti<'lua>,
        R: IntoLuaMulti<'lua>,
    {
        let name = name.as_ref();
        self.meta_methods
            .push((name.into(), Self::box_method(name, method)));
    }

    fn add_meta_method_mut<M, A, R>(&mut self, name: impl AsRef<str>, method: M)
    where
        M: FnMut(&'lua Lua, &mut T, A) -> Result<R> + MaybeSend + 'static,
        A: FromLuaMulti<'lua>,
        R: IntoLuaMulti<'lua>,
    {
        let name = name.as_ref();
        self.meta_methods
            .push((name.into(), Self::box_method_mut(name, method)));
    }

    #[cfg(all(feature = "async", not(any(feature = "lua51", feature = "luau"))))]
    fn add_async_meta_method<M, A, MR, R>(&mut self, name: impl AsRef<str>, method: M)
    where
        T: Clone,
        M: Fn(&'lua Lua, T, A) -> MR + MaybeSend + 'static,
        A: FromLuaMulti<'lua>,
        MR: Future<Output = Result<R>> + 'lua,
        R: IntoLuaMulti<'lua>,
    {
        let name = name.as_ref();
        self.async_meta_methods
            .push((name.into(), Self::box_async_method(name, method)));
    }

    fn add_meta_function<F, A, R>(&mut self, name: impl AsRef<str>, function: F)
    where
        F: Fn(&'lua Lua, A) -> Result<R> + MaybeSend + 'static,
        A: FromLuaMulti<'lua>,
        R: IntoLuaMulti<'lua>,
    {
        let name = name.as_ref();
        self.meta_methods
            .push((name.into(), Self::box_function(name, function)));
    }

    fn add_meta_function_mut<F, A, R>(&mut self, name: impl AsRef<str>, function: F)
    where
        F: FnMut(&'lua Lua, A) -> Result<R> + MaybeSend + 'static,
        A: FromLuaMulti<'lua>,
        R: IntoLuaMulti<'lua>,
    {
        let name = name.as_ref();
        self.meta_methods
            .push((name.into(), Self::box_function_mut(name, function)));
    }

    #[cfg(all(feature = "async", not(any(feature = "lua51", feature = "luau"))))]
    fn add_async_meta_function<F, A, FR, R>(&mut self, name: impl AsRef<str>, function: F)
    where
        F: Fn(&'lua Lua, A) -> FR + MaybeSend + 'static,
        A: FromLuaMulti<'lua>,
        FR: Future<Output = Result<R>> + 'lua,
        R: IntoLuaMulti<'lua>,
    {
        let name = name.as_ref();
        self.async_meta_methods
            .push((name.into(), Self::box_async_function(name, function)));
    }

    // Below are internal methods used in generated code

    fn add_callback(&mut self, name: String, callback: Callback<'lua, 'static>) {
        self.methods.push((name, callback));
    }

    #[cfg(feature = "async")]
    fn add_async_callback(&mut self, name: String, callback: AsyncCallback<'lua, 'static>) {
        self.async_methods.push((name, callback));
    }

    fn add_meta_callback(&mut self, name: String, callback: Callback<'lua, 'static>) {
        self.meta_methods.push((name, callback));
    }

    #[cfg(feature = "async")]
    fn add_async_meta_callback(&mut self, meta: String, callback: AsyncCallback<'lua, 'static>) {
        self.async_meta_methods.push((meta, callback))
    }
}

#[inline]
unsafe fn get_userdata_ref<'a, T>(state: *mut ffi::lua_State) -> Result<Ref<'a, T>> {
    (*get_userdata::<UserDataCell<T>>(state, -1)).try_borrow()
}

#[inline]
unsafe fn get_userdata_mut<'a, T>(state: *mut ffi::lua_State) -> Result<RefMut<'a, T>> {
    (*get_userdata::<UserDataCell<T>>(state, -1)).try_borrow_mut()
}

macro_rules! lua_userdata_impl {
    ($type:ty) => {
        impl<T: UserData + 'static> UserData for $type {
            fn add_fields<'lua, F: UserDataFields<'lua, Self>>(fields: &mut F) {
                let mut orig_fields = UserDataRegistrar::new();
                T::add_fields(&mut orig_fields);
                for (name, callback) in orig_fields.field_getters {
                    fields.add_field_getter(name, callback);
                }
                for (name, callback) in orig_fields.field_setters {
                    fields.add_field_setter(name, callback);
                }
            }

            fn add_methods<'lua, M: UserDataMethods<'lua, Self>>(methods: &mut M) {
                let mut orig_methods = UserDataRegistrar::new();
                T::add_methods(&mut orig_methods);
                for (name, callback) in orig_methods.methods {
                    methods.add_callback(name, callback);
                }
                #[cfg(feature = "async")]
                for (name, callback) in orig_methods.async_methods {
                    methods.add_async_callback(name, callback);
                }
                for (meta, callback) in orig_methods.meta_methods {
                    methods.add_meta_callback(meta, callback);
                }
                #[cfg(feature = "async")]
                for (meta, callback) in orig_methods.async_meta_methods {
                    methods.add_async_meta_callback(meta, callback);
                }
            }
        }
    };
}

#[cfg(not(feature = "send"))]
lua_userdata_impl!(Rc<RefCell<T>>);
lua_userdata_impl!(Arc<Mutex<T>>);
lua_userdata_impl!(Arc<RwLock<T>>);
#[cfg(feature = "parking_lot")]
lua_userdata_impl!(Arc<parking_lot::Mutex<T>>);
#[cfg(feature = "parking_lot")]
lua_userdata_impl!(Arc<parking_lot::RwLock<T>>);

// A special proxy object for UserData
pub(crate) struct UserDataProxy<T>(pub(crate) PhantomData<T>);

lua_userdata_impl!(UserDataProxy<T>);
