#![allow(clippy::await_holding_refcell_ref, clippy::await_holding_lock)]

use std::any::TypeId;
use std::cell::{Ref, RefCell, RefMut};
use std::marker::PhantomData;
use std::os::raw::c_int;
use std::string::String as StdString;
use std::sync::{Arc, Mutex, RwLock};

use crate::error::{Error, Result};
use crate::lua::Lua;
use crate::types::{Callback, MaybeSend};
use crate::userdata::{
    AnyUserData, MetaMethod, UserData, UserDataCell, UserDataFields, UserDataMethods,
};
use crate::util::{get_userdata, short_type_name};
use crate::value::{FromLua, FromLuaMulti, IntoLua, IntoLuaMulti, MultiValue, Value};

#[cfg(not(feature = "send"))]
use std::rc::Rc;

#[cfg(feature = "async")]
use {
    crate::types::AsyncCallback,
    futures_util::future::{self, TryFutureExt},
    std::future::Future,
};

/// Handle to registry for userdata methods and metamethods.
pub struct UserDataRegistry<'lua, T: 'static> {
    // Fields
    pub(crate) fields: Vec<(String, Callback<'lua, 'static>)>,
    pub(crate) field_getters: Vec<(String, Callback<'lua, 'static>)>,
    pub(crate) field_setters: Vec<(String, Callback<'lua, 'static>)>,
    pub(crate) meta_fields: Vec<(String, Callback<'lua, 'static>)>,

    // Methods
    pub(crate) methods: Vec<(String, Callback<'lua, 'static>)>,
    #[cfg(feature = "async")]
    pub(crate) async_methods: Vec<(String, AsyncCallback<'lua, 'static>)>,
    pub(crate) meta_methods: Vec<(String, Callback<'lua, 'static>)>,
    #[cfg(feature = "async")]
    pub(crate) async_meta_methods: Vec<(String, AsyncCallback<'lua, 'static>)>,

    _type: PhantomData<T>,
}

impl<'lua, T: 'static> UserDataRegistry<'lua, T> {
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
            let front = args
                .pop_front()
                .ok_or_else(|| Error::from_lua_conversion("missing argument", "userdata", None));
            let front = try_self_arg!(front);
            let call = |ud| {
                // Self was at index 1, so we pass 2 here
                let args = A::from_lua_multi_args(args, 2, Some(&name), lua)?;
                method(lua, ud, args)?.into_lua_multi(lua)
            };

            let userdata = try_self_arg!(AnyUserData::from_lua(front, lua));
            let (ref_thread, index) = (lua.ref_thread(), userdata.0.index);
            match try_self_arg!(userdata.type_id()) {
                Some(id) if id == TypeId::of::<T>() => unsafe {
                    let ud = try_self_arg!(get_userdata_ref::<T>(ref_thread, index));
                    call(&ud)
                },
                #[cfg(not(feature = "send"))]
                Some(id) if id == TypeId::of::<Rc<T>>() => unsafe {
                    let ud = try_self_arg!(get_userdata_ref::<Rc<T>>(ref_thread, index));
                    call(&ud)
                },
                #[cfg(not(feature = "send"))]
                Some(id) if id == TypeId::of::<Rc<RefCell<T>>>() => unsafe {
                    let ud = try_self_arg!(get_userdata_ref::<Rc<RefCell<T>>>(ref_thread, index));
                    let ud = try_self_arg!(ud.try_borrow(), Error::UserDataBorrowError);
                    call(&ud)
                },
                Some(id) if id == TypeId::of::<Arc<T>>() => unsafe {
                    let ud = try_self_arg!(get_userdata_ref::<Arc<T>>(ref_thread, index));
                    call(&ud)
                },
                Some(id) if id == TypeId::of::<Arc<Mutex<T>>>() => unsafe {
                    let ud = try_self_arg!(get_userdata_ref::<Arc<Mutex<T>>>(ref_thread, index));
                    let ud = try_self_arg!(ud.try_lock(), Error::UserDataBorrowError);
                    call(&ud)
                },
                #[cfg(feature = "parking_lot")]
                Some(id) if id == TypeId::of::<Arc<parking_lot::Mutex<T>>>() => unsafe {
                    let ud = get_userdata_ref::<Arc<parking_lot::Mutex<T>>>(ref_thread, index);
                    let ud = try_self_arg!(ud);
                    let ud = try_self_arg!(ud.try_lock().ok_or(Error::UserDataBorrowError));
                    call(&ud)
                },
                Some(id) if id == TypeId::of::<Arc<RwLock<T>>>() => unsafe {
                    let ud = try_self_arg!(get_userdata_ref::<Arc<RwLock<T>>>(ref_thread, index));
                    let ud = try_self_arg!(ud.try_read(), Error::UserDataBorrowError);
                    call(&ud)
                },
                #[cfg(feature = "parking_lot")]
                Some(id) if id == TypeId::of::<Arc<parking_lot::RwLock<T>>>() => unsafe {
                    let ud = get_userdata_ref::<Arc<parking_lot::RwLock<T>>>(ref_thread, index);
                    let ud = try_self_arg!(ud);
                    let ud = try_self_arg!(ud.try_read().ok_or(Error::UserDataBorrowError));
                    call(&ud)
                },
                _ => Err(Error::bad_self_argument(&name, Error::UserDataTypeMismatch)),
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
            let front = args
                .pop_front()
                .ok_or_else(|| Error::from_lua_conversion("missing argument", "userdata", None));
            let front = try_self_arg!(front);
            let call = |ud| {
                // Self was at index 1, so we pass 2 here
                let args = A::from_lua_multi_args(args, 2, Some(&name), lua)?;
                method(lua, ud, args)?.into_lua_multi(lua)
            };

            let userdata = try_self_arg!(AnyUserData::from_lua(front, lua));
            let (ref_thread, index) = (lua.ref_thread(), userdata.0.index);
            match try_self_arg!(userdata.type_id()) {
                Some(id) if id == TypeId::of::<T>() => unsafe {
                    let mut ud = try_self_arg!(get_userdata_mut::<T>(ref_thread, index));
                    call(&mut ud)
                },
                #[cfg(not(feature = "send"))]
                Some(id) if id == TypeId::of::<Rc<T>>() => Err(Error::UserDataBorrowMutError),
                #[cfg(not(feature = "send"))]
                Some(id) if id == TypeId::of::<Rc<RefCell<T>>>() => unsafe {
                    let ud = try_self_arg!(get_userdata_mut::<Rc<RefCell<T>>>(ref_thread, index));
                    let mut ud = try_self_arg!(ud.try_borrow_mut(), Error::UserDataBorrowMutError);
                    call(&mut ud)
                },
                Some(id) if id == TypeId::of::<Arc<T>>() => Err(Error::UserDataBorrowMutError),
                Some(id) if id == TypeId::of::<Arc<Mutex<T>>>() => unsafe {
                    let ud = try_self_arg!(get_userdata_mut::<Arc<Mutex<T>>>(ref_thread, index));
                    let mut ud = try_self_arg!(ud.try_lock(), Error::UserDataBorrowMutError);
                    call(&mut ud)
                },
                #[cfg(feature = "parking_lot")]
                Some(id) if id == TypeId::of::<Arc<parking_lot::Mutex<T>>>() => unsafe {
                    let ud = get_userdata_mut::<Arc<parking_lot::Mutex<T>>>(ref_thread, index);
                    let ud = try_self_arg!(ud);
                    let mut ud = try_self_arg!(ud.try_lock().ok_or(Error::UserDataBorrowMutError));
                    call(&mut ud)
                },
                Some(id) if id == TypeId::of::<Arc<RwLock<T>>>() => unsafe {
                    let ud = try_self_arg!(get_userdata_mut::<Arc<RwLock<T>>>(ref_thread, index));
                    let mut ud = try_self_arg!(ud.try_write(), Error::UserDataBorrowMutError);
                    call(&mut ud)
                },
                #[cfg(feature = "parking_lot")]
                Some(id) if id == TypeId::of::<Arc<parking_lot::RwLock<T>>>() => unsafe {
                    let ud = get_userdata_mut::<Arc<parking_lot::RwLock<T>>>(ref_thread, index);
                    let ud = try_self_arg!(ud);
                    let mut ud = try_self_arg!(ud.try_write().ok_or(Error::UserDataBorrowMutError));
                    call(&mut ud)
                },
                _ => Err(Error::bad_self_argument(&name, Error::UserDataTypeMismatch)),
            }
        })
    }

    #[cfg(feature = "async")]
    fn box_async_method<'s, M, A, MR, R>(name: &str, method: M) -> AsyncCallback<'lua, 'static>
    where
        'lua: 's,
        T: 'static,
        M: Fn(&'lua Lua, &'s T, A) -> MR + MaybeSend + 'static,
        A: FromLuaMulti<'lua>,
        MR: Future<Output = Result<R>> + 's,
        R: IntoLuaMulti<'lua>,
    {
        let name = get_function_name::<T>(name);
        let method = Arc::new(method);

        Box::new(move |lua, mut args| {
            let name = name.clone();
            let method = method.clone();
            macro_rules! try_self_arg {
                ($res:expr) => {
                    $res.map_err(|err| Error::bad_self_argument(&name, err))?
                };
                ($res:expr, $err:expr) => {
                    $res.map_err(|_| Error::bad_self_argument(&name, $err))?
                };
            }

            Box::pin(async move {
                let front = args.pop_front().ok_or_else(|| {
                    Error::from_lua_conversion("missing argument", "userdata", None)
                });
                let front = try_self_arg!(front);
                let userdata: AnyUserData = try_self_arg!(AnyUserData::from_lua(front, lua));
                let (ref_thread, index) = (lua.ref_thread(), userdata.0.index);
                match try_self_arg!(userdata.type_id()) {
                    Some(id) if id == TypeId::of::<T>() => unsafe {
                        let ud = try_self_arg!(get_userdata_ref::<T>(ref_thread, index));
                        let ud = std::mem::transmute::<&T, &T>(&ud);
                        // Self was at index 1, so we pass 2 here
                        let args = A::from_lua_multi_args(args, 2, Some(&name), lua)?;
                        method(lua, ud, args).await?.into_lua_multi(lua)
                    },
                    #[cfg(not(feature = "send"))]
                    Some(id) if id == TypeId::of::<Rc<T>>() => unsafe {
                        let ud = try_self_arg!(get_userdata_ref::<Rc<T>>(ref_thread, index));
                        let ud = std::mem::transmute::<&T, &T>(&ud);
                        let args = A::from_lua_multi_args(args, 2, Some(&name), lua)?;
                        method(lua, ud, args).await?.into_lua_multi(lua)
                    },
                    #[cfg(not(feature = "send"))]
                    Some(id) if id == TypeId::of::<Rc<RefCell<T>>>() => unsafe {
                        let ud =
                            try_self_arg!(get_userdata_ref::<Rc<RefCell<T>>>(ref_thread, index));
                        let ud = try_self_arg!(ud.try_borrow(), Error::UserDataBorrowError);
                        let ud = std::mem::transmute::<&T, &T>(&ud);
                        let args = A::from_lua_multi_args(args, 2, Some(&name), lua)?;
                        method(lua, ud, args).await?.into_lua_multi(lua)
                    },
                    Some(id) if id == TypeId::of::<Arc<T>>() => unsafe {
                        let ud = try_self_arg!(get_userdata_ref::<Arc<T>>(ref_thread, index));
                        let ud = std::mem::transmute::<&T, &T>(&ud);
                        let args = A::from_lua_multi_args(args, 2, Some(&name), lua)?;
                        method(lua, ud, args).await?.into_lua_multi(lua)
                    },
                    Some(id) if id == TypeId::of::<Arc<Mutex<T>>>() => unsafe {
                        let ud =
                            try_self_arg!(get_userdata_ref::<Arc<Mutex<T>>>(ref_thread, index));
                        let ud = try_self_arg!(ud.try_lock(), Error::UserDataBorrowError);
                        let ud = std::mem::transmute::<&T, &T>(&ud);
                        let args = A::from_lua_multi_args(args, 2, Some(&name), lua)?;
                        method(lua, ud, args).await?.into_lua_multi(lua)
                    },
                    #[cfg(feature = "parking_lot")]
                    Some(id) if id == TypeId::of::<Arc<parking_lot::Mutex<T>>>() => unsafe {
                        let ud = get_userdata_ref::<Arc<parking_lot::Mutex<T>>>(ref_thread, index);
                        let ud = try_self_arg!(ud);
                        let ud = try_self_arg!(ud.try_lock().ok_or(Error::UserDataBorrowError));
                        let ud = std::mem::transmute::<&T, &T>(&ud);
                        let args = A::from_lua_multi_args(args, 2, Some(&name), lua)?;
                        method(lua, ud, args).await?.into_lua_multi(lua)
                    },
                    Some(id) if id == TypeId::of::<Arc<RwLock<T>>>() => unsafe {
                        let ud =
                            try_self_arg!(get_userdata_ref::<Arc<RwLock<T>>>(ref_thread, index));
                        let ud = try_self_arg!(ud.try_read(), Error::UserDataBorrowError);
                        let ud = std::mem::transmute::<&T, &T>(&ud);
                        let args = A::from_lua_multi_args(args, 2, Some(&name), lua)?;
                        method(lua, ud, args).await?.into_lua_multi(lua)
                    },
                    #[cfg(feature = "parking_lot")]
                    Some(id) if id == TypeId::of::<Arc<parking_lot::RwLock<T>>>() => unsafe {
                        let ud = get_userdata_ref::<Arc<parking_lot::RwLock<T>>>(ref_thread, index);
                        let ud = try_self_arg!(ud);
                        let ud = try_self_arg!(ud.try_read().ok_or(Error::UserDataBorrowError));
                        let ud = std::mem::transmute::<&T, &T>(&ud);
                        let args = A::from_lua_multi_args(args, 2, Some(&name), lua)?;
                        method(lua, ud, args).await?.into_lua_multi(lua)
                    },
                    _ => Err(Error::bad_self_argument(&name, Error::UserDataTypeMismatch)),
                }
            })
        })
    }

    #[cfg(feature = "async")]
    fn box_async_method_mut<'s, M, A, MR, R>(name: &str, method: M) -> AsyncCallback<'lua, 'static>
    where
        'lua: 's,
        T: 'static,
        M: Fn(&'lua Lua, &'s mut T, A) -> MR + MaybeSend + 'static,
        A: FromLuaMulti<'lua>,
        MR: Future<Output = Result<R>> + 's,
        R: IntoLuaMulti<'lua>,
    {
        let name = get_function_name::<T>(name);
        let method = Arc::new(method);

        Box::new(move |lua, mut args| {
            let name = name.clone();
            let method = method.clone();
            macro_rules! try_self_arg {
                ($res:expr) => {
                    $res.map_err(|err| Error::bad_self_argument(&name, err))?
                };
                ($res:expr, $err:expr) => {
                    $res.map_err(|_| Error::bad_self_argument(&name, $err))?
                };
            }

            Box::pin(async move {
                let front = args.pop_front().ok_or_else(|| {
                    Error::from_lua_conversion("missing argument", "userdata", None)
                });
                let front = try_self_arg!(front);
                let userdata: AnyUserData = try_self_arg!(AnyUserData::from_lua(front, lua));
                let (ref_thread, index) = (lua.ref_thread(), userdata.0.index);
                match try_self_arg!(userdata.type_id()) {
                    Some(id) if id == TypeId::of::<T>() => unsafe {
                        let mut ud = try_self_arg!(get_userdata_mut::<T>(ref_thread, index));
                        let ud = std::mem::transmute::<&mut T, &mut T>(&mut ud);
                        // Self was at index 1, so we pass 2 here
                        let args = A::from_lua_multi_args(args, 2, Some(&name), lua)?;
                        method(lua, ud, args).await?.into_lua_multi(lua)
                    },
                    #[cfg(not(feature = "send"))]
                    Some(id) if id == TypeId::of::<Rc<RefCell<T>>>() => {
                        Err(Error::UserDataBorrowMutError)
                    }
                    #[cfg(not(feature = "send"))]
                    Some(id) if id == TypeId::of::<Rc<RefCell<T>>>() => unsafe {
                        let ud =
                            try_self_arg!(get_userdata_mut::<Rc<RefCell<T>>>(ref_thread, index));
                        let mut ud =
                            try_self_arg!(ud.try_borrow_mut(), Error::UserDataBorrowMutError);
                        let ud = std::mem::transmute::<&mut T, &mut T>(&mut ud);
                        let args = A::from_lua_multi_args(args, 2, Some(&name), lua)?;
                        method(lua, ud, args).await?.into_lua_multi(lua)
                    },
                    #[cfg(not(feature = "send"))]
                    Some(id) if id == TypeId::of::<Arc<T>>() => Err(Error::UserDataBorrowMutError),
                    Some(id) if id == TypeId::of::<Arc<Mutex<T>>>() => unsafe {
                        let ud =
                            try_self_arg!(get_userdata_mut::<Arc<Mutex<T>>>(ref_thread, index));
                        let mut ud = try_self_arg!(ud.try_lock(), Error::UserDataBorrowMutError);
                        let ud = std::mem::transmute::<&mut T, &mut T>(&mut ud);
                        let args = A::from_lua_multi_args(args, 2, Some(&name), lua)?;
                        method(lua, ud, args).await?.into_lua_multi(lua)
                    },
                    #[cfg(feature = "parking_lot")]
                    Some(id) if id == TypeId::of::<Arc<parking_lot::Mutex<T>>>() => unsafe {
                        let ud = get_userdata_mut::<Arc<parking_lot::Mutex<T>>>(ref_thread, index);
                        let ud = try_self_arg!(ud);
                        let mut ud =
                            try_self_arg!(ud.try_lock().ok_or(Error::UserDataBorrowMutError));
                        let ud = std::mem::transmute::<&mut T, &mut T>(&mut ud);
                        let args = A::from_lua_multi_args(args, 2, Some(&name), lua)?;
                        method(lua, ud, args).await?.into_lua_multi(lua)
                    },
                    Some(id) if id == TypeId::of::<Arc<RwLock<T>>>() => unsafe {
                        let ud =
                            try_self_arg!(get_userdata_mut::<Arc<RwLock<T>>>(ref_thread, index));
                        let mut ud = try_self_arg!(ud.try_write(), Error::UserDataBorrowMutError);
                        let ud = std::mem::transmute::<&mut T, &mut T>(&mut ud);
                        let args = A::from_lua_multi_args(args, 2, Some(&name), lua)?;
                        method(lua, ud, args).await?.into_lua_multi(lua)
                    },
                    #[cfg(feature = "parking_lot")]
                    Some(id) if id == TypeId::of::<Arc<parking_lot::RwLock<T>>>() => unsafe {
                        let ud = get_userdata_mut::<Arc<parking_lot::RwLock<T>>>(ref_thread, index);
                        let ud = try_self_arg!(ud);
                        let mut ud =
                            try_self_arg!(ud.try_write().ok_or(Error::UserDataBorrowMutError));
                        let ud = std::mem::transmute::<&mut T, &mut T>(&mut ud);
                        let args = A::from_lua_multi_args(args, 2, Some(&name), lua)?;
                        method(lua, ud, args).await?.into_lua_multi(lua)
                    },
                    _ => Err(Error::bad_self_argument(&name, Error::UserDataTypeMismatch)),
                }
            })
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

    pub(crate) fn check_meta_field<V>(
        lua: &'lua Lua,
        name: &str,
        value: V,
    ) -> Result<MultiValue<'lua>>
    where
        V: IntoLua<'lua>,
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
        value.into_lua_multi(lua)
    }
}

// Returns function name for the type `T`, without the module path
fn get_function_name<T>(name: &str) -> StdString {
    format!("{}.{name}", short_type_name::<T>())
}

impl<'lua, T: 'static> UserDataFields<'lua, T> for UserDataRegistry<'lua, T> {
    fn add_field<V>(&mut self, name: impl AsRef<str>, value: V)
    where
        V: IntoLua<'lua> + Clone + 'static,
    {
        let name = name.as_ref().to_string();
        self.fields.push((
            name,
            Box::new(move |lua, _| value.clone().into_lua_multi(lua)),
        ));
    }

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

    fn add_meta_field<V>(&mut self, name: impl AsRef<str>, value: V)
    where
        V: IntoLua<'lua> + Clone + 'static,
    {
        let name = name.as_ref().to_string();
        let name2 = name.clone();
        self.meta_fields.push((
            name,
            Box::new(move |lua, _| Self::check_meta_field(lua, &name2, value.clone())),
        ));
    }

    fn add_meta_field_with<F, R>(&mut self, name: impl AsRef<str>, f: F)
    where
        F: Fn(&'lua Lua) -> Result<R> + MaybeSend + 'static,
        R: IntoLua<'lua>,
    {
        let name = name.as_ref().to_string();
        let name2 = name.clone();
        self.meta_fields.push((
            name,
            Box::new(move |lua, _| Self::check_meta_field(lua, &name2, f(lua)?)),
        ));
    }

    // Below are internal methods

    fn append_fields_from<S>(&mut self, other: UserDataRegistry<'lua, S>) {
        self.fields.extend(other.fields);
        self.field_getters.extend(other.field_getters);
        self.field_setters.extend(other.field_setters);
        self.meta_fields.extend(other.meta_fields);
    }
}

impl<'lua, T: 'static> UserDataMethods<'lua, T> for UserDataRegistry<'lua, T> {
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
    fn add_async_method<'s, M, A, MR, R>(&mut self, name: impl AsRef<str>, method: M)
    where
        'lua: 's,
        T: 'static,
        M: Fn(&'lua Lua, &'s T, A) -> MR + MaybeSend + 'static,
        A: FromLuaMulti<'lua>,
        MR: Future<Output = Result<R>> + 's,
        R: IntoLuaMulti<'lua>,
    {
        let name = name.as_ref();
        self.async_methods
            .push((name.into(), Self::box_async_method(name, method)));
    }

    #[cfg(feature = "async")]
    fn add_async_method_mut<'s, M, A, MR, R>(&mut self, name: impl AsRef<str>, method: M)
    where
        'lua: 's,
        T: 'static,
        M: Fn(&'lua Lua, &'s mut T, A) -> MR + MaybeSend + 'static,
        A: FromLuaMulti<'lua>,
        MR: Future<Output = Result<R>> + 's,
        R: IntoLuaMulti<'lua>,
    {
        let name = name.as_ref();
        self.async_methods
            .push((name.into(), Self::box_async_method_mut(name, method)));
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
    fn add_async_meta_method<'s, M, A, MR, R>(&mut self, name: impl AsRef<str>, method: M)
    where
        'lua: 's,
        T: 'static,
        M: Fn(&'lua Lua, &'s T, A) -> MR + MaybeSend + 'static,
        A: FromLuaMulti<'lua>,
        MR: Future<Output = Result<R>> + 's,
        R: IntoLuaMulti<'lua>,
    {
        let name = name.as_ref();
        self.async_meta_methods
            .push((name.into(), Self::box_async_method(name, method)));
    }

    #[cfg(all(feature = "async", not(any(feature = "lua51", feature = "luau"))))]
    fn add_async_meta_method_mut<'s, M, A, MR, R>(&mut self, name: impl AsRef<str>, method: M)
    where
        'lua: 's,
        T: 'static,
        M: Fn(&'lua Lua, &'s mut T, A) -> MR + MaybeSend + 'static,
        A: FromLuaMulti<'lua>,
        MR: Future<Output = Result<R>> + 's,
        R: IntoLuaMulti<'lua>,
    {
        let name = name.as_ref();
        self.async_meta_methods
            .push((name.into(), Self::box_async_method_mut(name, method)));
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

    fn append_methods_from<S>(&mut self, other: UserDataRegistry<'lua, S>) {
        self.methods.extend(other.methods);
        #[cfg(feature = "async")]
        self.async_methods.extend(other.async_methods);
        self.meta_methods.extend(other.meta_methods);
        #[cfg(feature = "async")]
        self.async_meta_methods.extend(other.async_meta_methods);
    }
}

#[inline]
unsafe fn get_userdata_ref<'a, T>(state: *mut ffi::lua_State, index: c_int) -> Result<Ref<'a, T>> {
    (*get_userdata::<UserDataCell<T>>(state, index)).try_borrow()
}

#[inline]
unsafe fn get_userdata_mut<'a, T>(
    state: *mut ffi::lua_State,
    index: c_int,
) -> Result<RefMut<'a, T>> {
    (*get_userdata::<UserDataCell<T>>(state, index)).try_borrow_mut()
}

macro_rules! lua_userdata_impl {
    ($type:ty) => {
        impl<T: UserData + 'static> UserData for $type {
            fn add_fields<'lua, F: UserDataFields<'lua, Self>>(fields: &mut F) {
                let mut orig_fields = UserDataRegistry::new();
                T::add_fields(&mut orig_fields);
                fields.append_fields_from(orig_fields);
            }

            fn add_methods<'lua, M: UserDataMethods<'lua, Self>>(methods: &mut M) {
                let mut orig_methods = UserDataRegistry::new();
                T::add_methods(&mut orig_methods);
                methods.append_methods_from(orig_methods);
            }
        }
    };
}

#[cfg(not(feature = "send"))]
lua_userdata_impl!(Rc<T>);
#[cfg(not(feature = "send"))]
lua_userdata_impl!(Rc<RefCell<T>>);

lua_userdata_impl!(Arc<T>);
lua_userdata_impl!(Arc<Mutex<T>>);
lua_userdata_impl!(Arc<RwLock<T>>);
#[cfg(feature = "parking_lot")]
lua_userdata_impl!(Arc<parking_lot::Mutex<T>>);
#[cfg(feature = "parking_lot")]
lua_userdata_impl!(Arc<parking_lot::RwLock<T>>);

// A special proxy object for UserData
pub(crate) struct UserDataProxy<T>(pub(crate) PhantomData<T>);

lua_userdata_impl!(UserDataProxy<T>);
