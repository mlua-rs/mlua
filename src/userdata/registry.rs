#![allow(clippy::await_holding_refcell_ref, clippy::await_holding_lock)]

use std::any::TypeId;
use std::cell::RefCell;
use std::marker::PhantomData;
use std::os::raw::c_void;
use std::string::String as StdString;

use crate::error::{Error, Result};
use crate::state::{Lua, RawLua};
use crate::types::{Callback, MaybeSend};
use crate::userdata::{
    AnyUserData, MetaMethod, UserData, UserDataFields, UserDataMethods, UserDataRef, UserDataRefMut,
    UserDataStorage,
};
use crate::util::{get_userdata, short_type_name};
use crate::value::{FromLua, FromLuaMulti, IntoLua, IntoLuaMulti, Value};

#[cfg(feature = "async")]
use {
    crate::types::AsyncCallback,
    std::future::{self, Future},
};

type StaticFieldCallback = Box<dyn FnOnce(&RawLua) -> Result<()> + 'static>;

#[derive(Clone, Copy)]
pub(crate) enum UserDataTypeId {
    Shared(TypeId),
    Unique(usize),
}

/// Handle to registry for userdata methods and metamethods.
pub struct UserDataRegistry<T> {
    // Fields
    pub(crate) fields: Vec<(String, StaticFieldCallback)>,
    pub(crate) field_getters: Vec<(String, Callback)>,
    pub(crate) field_setters: Vec<(String, Callback)>,
    pub(crate) meta_fields: Vec<(String, StaticFieldCallback)>,

    // Methods
    pub(crate) methods: Vec<(String, Callback)>,
    #[cfg(feature = "async")]
    pub(crate) async_methods: Vec<(String, AsyncCallback)>,
    pub(crate) meta_methods: Vec<(String, Callback)>,
    #[cfg(feature = "async")]
    pub(crate) async_meta_methods: Vec<(String, AsyncCallback)>,

    pub(crate) type_id: UserDataTypeId,
    _type: PhantomData<T>,
}

impl<T> UserDataRegistry<T> {
    #[inline]
    pub(crate) fn new(type_id: TypeId) -> Self {
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
            type_id: UserDataTypeId::Shared(type_id),
            _type: PhantomData,
        }
    }

    #[inline]
    pub(crate) fn new_unique(ud_ptr: *const c_void) -> Self {
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
            type_id: UserDataTypeId::Unique(ud_ptr as usize),
            _type: PhantomData,
        }
    }

    #[inline]
    pub(crate) fn type_id(&self) -> Option<TypeId> {
        match self.type_id {
            UserDataTypeId::Shared(type_id) => Some(type_id),
            UserDataTypeId::Unique(_) => None,
        }
    }

    fn box_method<M, A, R>(&self, name: &str, method: M) -> Callback
    where
        M: Fn(&Lua, &T, A) -> Result<R> + MaybeSend + 'static,
        A: FromLuaMulti,
        R: IntoLuaMulti,
    {
        let name = get_function_name::<T>(name);
        macro_rules! try_self_arg {
            ($res:expr) => {
                $res.map_err(|err| Error::bad_self_argument(&name, err))?
            };
        }

        let target_type_id = self.type_id;
        Box::new(move |rawlua, nargs| unsafe {
            if nargs == 0 {
                let err = Error::from_lua_conversion("missing argument", "userdata", None);
                try_self_arg!(Err(err));
            }
            let state = rawlua.state();
            // Find absolute "self" index before processing args
            let self_index = ffi::lua_absindex(state, -nargs);
            // Self was at position 1, so we pass 2 here
            let args = A::from_stack_args(nargs - 1, 2, Some(&name), rawlua);

            match target_type_id {
                // This branch is for `'static` userdata that share type metatable
                UserDataTypeId::Shared(target_type_id) => {
                    match try_self_arg!(rawlua.get_userdata_type_id(self_index)) {
                        Some(self_type_id) if self_type_id == target_type_id => {
                            let ud = get_userdata::<UserDataStorage<T>>(state, self_index);
                            try_self_arg!((*ud).try_borrow_scoped(|ud| {
                                method(rawlua.lua(), ud, args?)?.push_into_stack_multi(rawlua)
                            }))
                        }
                        _ => Err(Error::bad_self_argument(&name, Error::UserDataTypeMismatch)),
                    }
                }
                UserDataTypeId::Unique(target_ptr) => {
                    match get_userdata::<UserDataStorage<T>>(state, self_index) {
                        ud if ud as usize == target_ptr => {
                            try_self_arg!((*ud).try_borrow_scoped(|ud| {
                                method(rawlua.lua(), ud, args?)?.push_into_stack_multi(rawlua)
                            }))
                        }
                        _ => Err(Error::bad_self_argument(&name, Error::UserDataTypeMismatch)),
                    }
                }
            }
        })
    }

    fn box_method_mut<M, A, R>(&self, name: &str, method: M) -> Callback
    where
        M: FnMut(&Lua, &mut T, A) -> Result<R> + MaybeSend + 'static,
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
        let target_type_id = self.type_id;
        Box::new(move |rawlua, nargs| unsafe {
            let mut method = method.try_borrow_mut().map_err(|_| Error::RecursiveMutCallback)?;
            if nargs == 0 {
                let err = Error::from_lua_conversion("missing argument", "userdata", None);
                try_self_arg!(Err(err));
            }
            let state = rawlua.state();
            // Find absolute "self" index before processing args
            let self_index = ffi::lua_absindex(state, -nargs);
            // Self was at position 1, so we pass 2 here
            let args = A::from_stack_args(nargs - 1, 2, Some(&name), rawlua);

            match target_type_id {
                // This branch is for `'static` userdata that share type metatable
                UserDataTypeId::Shared(target_type_id) => {
                    match try_self_arg!(rawlua.get_userdata_type_id(self_index)) {
                        Some(self_type_id) if self_type_id == target_type_id => {
                            let ud = get_userdata::<UserDataStorage<T>>(state, self_index);
                            try_self_arg!((*ud).try_borrow_scoped_mut(|ud| {
                                method(rawlua.lua(), ud, args?)?.push_into_stack_multi(rawlua)
                            }))
                        }
                        _ => Err(Error::bad_self_argument(&name, Error::UserDataTypeMismatch)),
                    }
                }
                UserDataTypeId::Unique(target_ptr) => {
                    match get_userdata::<UserDataStorage<T>>(state, self_index) {
                        ud if ud as usize == target_ptr => {
                            try_self_arg!((*ud).try_borrow_scoped_mut(|ud| {
                                method(rawlua.lua(), ud, args?)?.push_into_stack_multi(rawlua)
                            }))
                        }
                        _ => Err(Error::bad_self_argument(&name, Error::UserDataTypeMismatch)),
                    }
                }
            }
        })
    }

    #[cfg(feature = "async")]
    fn box_async_method<M, A, MR, R>(&self, name: &str, method: M) -> AsyncCallback
    where
        T: 'static,
        M: Fn(Lua, UserDataRef<T>, A) -> MR + MaybeSend + 'static,
        A: FromLuaMulti,
        MR: Future<Output = Result<R>> + MaybeSend + 'static,
        R: IntoLuaMulti,
    {
        let name = get_function_name::<T>(name);
        macro_rules! try_self_arg {
            ($res:expr) => {
                match $res {
                    Ok(res) => res,
                    Err(err) => return Box::pin(future::ready(Err(Error::bad_self_argument(&name, err)))),
                }
            };
        }

        Box::new(move |rawlua, nargs| unsafe {
            if nargs == 0 {
                let err = Error::from_lua_conversion("missing argument", "userdata", None);
                try_self_arg!(Err(err));
            }
            // Stack will be empty when polling the future, keep `self` on the ref thread
            let self_ud = try_self_arg!(AnyUserData::from_stack(-nargs, rawlua));
            let args = A::from_stack_args(nargs - 1, 2, Some(&name), rawlua);

            let self_ud = try_self_arg!(self_ud.borrow());
            let args = match args {
                Ok(args) => args,
                Err(e) => return Box::pin(future::ready(Err(e))),
            };
            let lua = rawlua.lua();
            let fut = method(lua.clone(), self_ud, args);
            // Lua is locked when the future is polled
            Box::pin(async move { fut.await?.push_into_stack_multi(lua.raw_lua()) })
        })
    }

    #[cfg(feature = "async")]
    fn box_async_method_mut<M, A, MR, R>(&self, name: &str, method: M) -> AsyncCallback
    where
        T: 'static,
        M: Fn(Lua, UserDataRefMut<T>, A) -> MR + MaybeSend + 'static,
        A: FromLuaMulti,
        MR: Future<Output = Result<R>> + MaybeSend + 'static,
        R: IntoLuaMulti,
    {
        let name = get_function_name::<T>(name);
        macro_rules! try_self_arg {
            ($res:expr) => {
                match $res {
                    Ok(res) => res,
                    Err(err) => return Box::pin(future::ready(Err(Error::bad_self_argument(&name, err)))),
                }
            };
        }

        Box::new(move |rawlua, nargs| unsafe {
            if nargs == 0 {
                let err = Error::from_lua_conversion("missing argument", "userdata", None);
                try_self_arg!(Err(err));
            }
            // Stack will be empty when polling the future, keep `self` on the ref thread
            let self_ud = try_self_arg!(AnyUserData::from_stack(-nargs, rawlua));
            let args = A::from_stack_args(nargs - 1, 2, Some(&name), rawlua);

            let self_ud = try_self_arg!(self_ud.borrow_mut());
            let args = match args {
                Ok(args) => args,
                Err(e) => return Box::pin(future::ready(Err(e))),
            };
            let lua = rawlua.lua();
            let fut = method(lua.clone(), self_ud, args);
            // Lua is locked when the future is polled
            Box::pin(async move { fut.await?.push_into_stack_multi(lua.raw_lua()) })
        })
    }

    fn box_function<F, A, R>(&self, name: &str, function: F) -> Callback
    where
        F: Fn(&Lua, A) -> Result<R> + MaybeSend + 'static,
        A: FromLuaMulti,
        R: IntoLuaMulti,
    {
        let name = get_function_name::<T>(name);
        Box::new(move |lua, nargs| unsafe {
            let args = A::from_stack_args(nargs, 1, Some(&name), lua)?;
            function(lua.lua(), args)?.push_into_stack_multi(lua)
        })
    }

    fn box_function_mut<F, A, R>(&self, name: &str, function: F) -> Callback
    where
        F: FnMut(&Lua, A) -> Result<R> + MaybeSend + 'static,
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
    fn box_async_function<F, A, FR, R>(&self, name: &str, function: F) -> AsyncCallback
    where
        F: Fn(Lua, A) -> FR + MaybeSend + 'static,
        A: FromLuaMulti,
        FR: Future<Output = Result<R>> + MaybeSend + 'static,
        R: IntoLuaMulti,
    {
        let name = get_function_name::<T>(name);
        Box::new(move |rawlua, nargs| unsafe {
            let args = match A::from_stack_args(nargs, 1, Some(&name), rawlua) {
                Ok(args) => args,
                Err(e) => return Box::pin(future::ready(Err(e))),
            };
            let lua = rawlua.lua();
            let fut = function(lua.clone(), args);
            Box::pin(async move { fut.await?.push_into_stack_multi(lua.raw_lua()) })
        })
    }

    pub(crate) fn check_meta_field(lua: &Lua, name: &str, value: impl IntoLua) -> Result<Value> {
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

impl<T> UserDataFields<T> for UserDataRegistry<T> {
    fn add_field<V>(&mut self, name: impl ToString, value: V)
    where
        V: IntoLua + 'static,
    {
        let name = name.to_string();
        self.fields.push((
            name,
            Box::new(move |rawlua| unsafe { value.push_into_stack(rawlua) }),
        ));
    }

    fn add_field_method_get<M, R>(&mut self, name: impl ToString, method: M)
    where
        M: Fn(&Lua, &T) -> Result<R> + MaybeSend + 'static,
        R: IntoLua,
    {
        let name = name.to_string();
        let callback = self.box_method(&name, move |lua, data, ()| method(lua, data));
        self.field_getters.push((name, callback));
    }

    fn add_field_method_set<M, A>(&mut self, name: impl ToString, method: M)
    where
        M: FnMut(&Lua, &mut T, A) -> Result<()> + MaybeSend + 'static,
        A: FromLua,
    {
        let name = name.to_string();
        let callback = self.box_method_mut(&name, method);
        self.field_setters.push((name, callback));
    }

    fn add_field_function_get<F, R>(&mut self, name: impl ToString, function: F)
    where
        F: Fn(&Lua, AnyUserData) -> Result<R> + MaybeSend + 'static,
        R: IntoLua,
    {
        let name = name.to_string();
        let callback = self.box_function(&name, function);
        self.field_getters.push((name, callback));
    }

    fn add_field_function_set<F, A>(&mut self, name: impl ToString, mut function: F)
    where
        F: FnMut(&Lua, AnyUserData, A) -> Result<()> + MaybeSend + 'static,
        A: FromLua,
    {
        let name = name.to_string();
        let callback = self.box_function_mut(&name, move |lua, (data, val)| function(lua, data, val));
        self.field_setters.push((name, callback));
    }

    fn add_meta_field<V>(&mut self, name: impl ToString, value: V)
    where
        V: IntoLua + 'static,
    {
        let name = name.to_string();
        self.meta_fields.push((
            name.clone(),
            Box::new(move |rawlua| unsafe {
                Self::check_meta_field(rawlua.lua(), &name, value)?.push_into_stack(rawlua)
            }),
        ));
    }

    fn add_meta_field_with<F, R>(&mut self, name: impl ToString, f: F)
    where
        F: FnOnce(&Lua) -> Result<R> + 'static,
        R: IntoLua,
    {
        let name = name.to_string();
        self.meta_fields.push((
            name.clone(),
            Box::new(move |rawlua| unsafe {
                let lua = rawlua.lua();
                Self::check_meta_field(lua, &name, f(lua)?)?.push_into_stack(rawlua)
            }),
        ));
    }
}

impl<T> UserDataMethods<T> for UserDataRegistry<T> {
    fn add_method<M, A, R>(&mut self, name: impl ToString, method: M)
    where
        M: Fn(&Lua, &T, A) -> Result<R> + MaybeSend + 'static,
        A: FromLuaMulti,
        R: IntoLuaMulti,
    {
        let name = name.to_string();
        let callback = self.box_method(&name, method);
        self.methods.push((name, callback));
    }

    fn add_method_mut<M, A, R>(&mut self, name: impl ToString, method: M)
    where
        M: FnMut(&Lua, &mut T, A) -> Result<R> + MaybeSend + 'static,
        A: FromLuaMulti,
        R: IntoLuaMulti,
    {
        let name = name.to_string();
        let callback = self.box_method_mut(&name, method);
        self.methods.push((name, callback));
    }

    #[cfg(feature = "async")]
    fn add_async_method<M, A, MR, R>(&mut self, name: impl ToString, method: M)
    where
        T: 'static,
        M: Fn(Lua, UserDataRef<T>, A) -> MR + MaybeSend + 'static,
        A: FromLuaMulti,
        MR: Future<Output = Result<R>> + MaybeSend + 'static,
        R: IntoLuaMulti,
    {
        let name = name.to_string();
        let callback = self.box_async_method(&name, method);
        self.async_methods.push((name, callback));
    }

    #[cfg(feature = "async")]
    fn add_async_method_mut<M, A, MR, R>(&mut self, name: impl ToString, method: M)
    where
        T: 'static,
        M: Fn(Lua, UserDataRefMut<T>, A) -> MR + MaybeSend + 'static,
        A: FromLuaMulti,
        MR: Future<Output = Result<R>> + MaybeSend + 'static,
        R: IntoLuaMulti,
    {
        let name = name.to_string();
        let callback = self.box_async_method_mut(&name, method);
        self.async_methods.push((name, callback));
    }

    fn add_function<F, A, R>(&mut self, name: impl ToString, function: F)
    where
        F: Fn(&Lua, A) -> Result<R> + MaybeSend + 'static,
        A: FromLuaMulti,
        R: IntoLuaMulti,
    {
        let name = name.to_string();
        let callback = self.box_function(&name, function);
        self.methods.push((name, callback));
    }

    fn add_function_mut<F, A, R>(&mut self, name: impl ToString, function: F)
    where
        F: FnMut(&Lua, A) -> Result<R> + MaybeSend + 'static,
        A: FromLuaMulti,
        R: IntoLuaMulti,
    {
        let name = name.to_string();
        let callback = self.box_function_mut(&name, function);
        self.methods.push((name, callback));
    }

    #[cfg(feature = "async")]
    fn add_async_function<F, A, FR, R>(&mut self, name: impl ToString, function: F)
    where
        F: Fn(Lua, A) -> FR + MaybeSend + 'static,
        A: FromLuaMulti,
        FR: Future<Output = Result<R>> + MaybeSend + 'static,
        R: IntoLuaMulti,
    {
        let name = name.to_string();
        let callback = self.box_async_function(&name, function);
        self.async_methods.push((name, callback));
    }

    fn add_meta_method<M, A, R>(&mut self, name: impl ToString, method: M)
    where
        M: Fn(&Lua, &T, A) -> Result<R> + MaybeSend + 'static,
        A: FromLuaMulti,
        R: IntoLuaMulti,
    {
        let name = name.to_string();
        let callback = self.box_method(&name, method);
        self.meta_methods.push((name, callback));
    }

    fn add_meta_method_mut<M, A, R>(&mut self, name: impl ToString, method: M)
    where
        M: FnMut(&Lua, &mut T, A) -> Result<R> + MaybeSend + 'static,
        A: FromLuaMulti,
        R: IntoLuaMulti,
    {
        let name = name.to_string();
        let callback = self.box_method_mut(&name, method);
        self.meta_methods.push((name, callback));
    }

    #[cfg(all(feature = "async", not(any(feature = "lua51", feature = "luau"))))]
    fn add_async_meta_method<M, A, MR, R>(&mut self, name: impl ToString, method: M)
    where
        T: 'static,
        M: Fn(Lua, UserDataRef<T>, A) -> MR + MaybeSend + 'static,
        A: FromLuaMulti,
        MR: Future<Output = Result<R>> + MaybeSend + 'static,
        R: IntoLuaMulti,
    {
        let name = name.to_string();
        let callback = self.box_async_method(&name, method);
        self.async_meta_methods.push((name, callback));
    }

    #[cfg(all(feature = "async", not(any(feature = "lua51", feature = "luau"))))]
    fn add_async_meta_method_mut<M, A, MR, R>(&mut self, name: impl ToString, method: M)
    where
        T: 'static,
        M: Fn(Lua, UserDataRefMut<T>, A) -> MR + MaybeSend + 'static,
        A: FromLuaMulti,
        MR: Future<Output = Result<R>> + MaybeSend + 'static,
        R: IntoLuaMulti,
    {
        let name = name.to_string();
        let callback = self.box_async_method_mut(&name, method);
        self.async_meta_methods.push((name, callback));
    }

    fn add_meta_function<F, A, R>(&mut self, name: impl ToString, function: F)
    where
        F: Fn(&Lua, A) -> Result<R> + MaybeSend + 'static,
        A: FromLuaMulti,
        R: IntoLuaMulti,
    {
        let name = name.to_string();
        let callback = self.box_function(&name, function);
        self.meta_methods.push((name, callback));
    }

    fn add_meta_function_mut<F, A, R>(&mut self, name: impl ToString, function: F)
    where
        F: FnMut(&Lua, A) -> Result<R> + MaybeSend + 'static,
        A: FromLuaMulti,
        R: IntoLuaMulti,
    {
        let name = name.to_string();
        let callback = self.box_function_mut(&name, function);
        self.meta_methods.push((name, callback));
    }

    #[cfg(all(feature = "async", not(any(feature = "lua51", feature = "luau"))))]
    fn add_async_meta_function<F, A, FR, R>(&mut self, name: impl ToString, function: F)
    where
        F: Fn(Lua, A) -> FR + MaybeSend + 'static,
        A: FromLuaMulti,
        FR: Future<Output = Result<R>> + MaybeSend + 'static,
        R: IntoLuaMulti,
    {
        let name = name.to_string();
        let callback = self.box_async_function(&name, function);
        self.async_meta_methods.push((name, callback));
    }
}

macro_rules! lua_userdata_impl {
    ($type:ty) => {
        impl<T: UserData + 'static> UserData for $type {
            fn register(registry: &mut UserDataRegistry<Self>) {
                let type_id = TypeId::of::<T>();
                let mut orig_registry = UserDataRegistry::new(type_id);
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
