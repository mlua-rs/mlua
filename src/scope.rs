use std::any::Any;
use std::cell::{Cell, RefCell};
use std::marker::PhantomData;
use std::mem;
use std::os::raw::{c_int, c_void};
use std::rc::Rc;

#[cfg(feature = "serialize")]
use serde::Serialize;

use crate::error::{Error, Result};
use crate::ffi;
use crate::function::Function;
use crate::lua::Lua;
use crate::types::{Callback, CallbackUpvalue, LuaRef, MaybeSend};
use crate::userdata::{
    AnyUserData, MetaMethod, UserData, UserDataCell, UserDataFields, UserDataMethods,
};
use crate::util::{
    assert_stack, check_stack, get_userdata, init_userdata_metatable, push_table, rawset_field,
    take_userdata, StackGuard,
};
use crate::value::{FromLua, FromLuaMulti, MultiValue, ToLua, ToLuaMulti, Value};

#[cfg(feature = "lua54")]
use crate::userdata::USER_VALUE_MAXSLOT;

#[cfg(feature = "async")]
use {
    crate::types::{AsyncCallback, AsyncCallbackUpvalue, AsyncPollUpvalue},
    futures_core::future::Future,
    futures_util::future::{self, TryFutureExt},
};

/// Constructed by the [`Lua::scope`] method, allows temporarily creating Lua userdata and
/// callbacks that are not required to be Send or 'static.
///
/// See [`Lua::scope`] for more details.
///
/// [`Lua::scope`]: crate::Lua.html::scope
pub struct Scope<'lua, 'scope> {
    lua: &'lua Lua,
    destructors: RefCell<Vec<(LuaRef<'lua>, DestructorCallback<'lua>)>>,
    _scope_invariant: PhantomData<Cell<&'scope ()>>,
}

type DestructorCallback<'lua> = Box<dyn Fn(LuaRef<'lua>) -> Vec<Box<dyn Any>> + 'lua>;

impl<'lua, 'scope> Scope<'lua, 'scope> {
    pub(crate) fn new(lua: &'lua Lua) -> Scope<'lua, 'scope> {
        Scope {
            lua,
            destructors: RefCell::new(Vec::new()),
            _scope_invariant: PhantomData,
        }
    }

    /// Wraps a Rust function or closure, creating a callable Lua function handle to it.
    ///
    /// This is a version of [`Lua::create_function`] that creates a callback which expires on
    /// scope drop. See [`Lua::scope`] for more details.
    ///
    /// [`Lua::create_function`]: crate::Lua::create_function
    /// [`Lua::scope`]: crate::Lua::scope
    pub fn create_function<'callback, A, R, F>(&'callback self, func: F) -> Result<Function<'lua>>
    where
        A: FromLuaMulti<'callback>,
        R: ToLuaMulti<'callback>,
        F: 'scope + Fn(&'callback Lua, A) -> Result<R>,
    {
        // Safe, because 'scope must outlive 'callback (due to Self containing 'scope), however the
        // callback itself must be 'scope lifetime, so the function should not be able to capture
        // anything of 'callback lifetime. 'scope can't be shortened due to being invariant, and
        // the 'callback lifetime here can't be enlarged due to coming from a universal
        // quantification in Lua::scope.
        //
        // I hope I got this explanation right, but in any case this is tested with compiletest_rs
        // to make sure callbacks can't capture handles with lifetime outside the scope, inside the
        // scope, and owned inside the callback itself.
        unsafe {
            self.create_callback(Box::new(move |lua, args| {
                func(lua, A::from_lua_multi(args, lua)?)?.to_lua_multi(lua)
            }))
        }
    }

    /// Wraps a Rust mutable closure, creating a callable Lua function handle to it.
    ///
    /// This is a version of [`Lua::create_function_mut`] that creates a callback which expires
    /// on scope drop. See [`Lua::scope`] and [`Scope::create_function`] for more details.
    ///
    /// [`Lua::create_function_mut`]: crate::Lua::create_function_mut
    /// [`Lua::scope`]: crate::Lua::scope
    /// [`Scope::create_function`]: #method.create_function
    pub fn create_function_mut<'callback, A, R, F>(
        &'callback self,
        func: F,
    ) -> Result<Function<'lua>>
    where
        A: FromLuaMulti<'callback>,
        R: ToLuaMulti<'callback>,
        F: 'scope + FnMut(&'callback Lua, A) -> Result<R>,
    {
        let func = RefCell::new(func);
        self.create_function(move |lua, args| {
            (*func
                .try_borrow_mut()
                .map_err(|_| Error::RecursiveMutCallback)?)(lua, args)
        })
    }

    /// Wraps a Rust async function or closure, creating a callable Lua function handle to it.
    ///
    /// This is a version of [`Lua::create_async_function`] that creates a callback which expires on
    /// scope drop. See [`Lua::scope`] and [`Lua::async_scope`] for more details.
    ///
    /// Requires `feature = "async"`
    ///
    /// [`Lua::create_async_function`]: crate::Lua::create_async_function
    /// [`Lua::scope`]: crate::Lua::scope
    /// [`Lua::async_scope`]: crate::Lua::async_scope
    #[cfg(feature = "async")]
    #[cfg_attr(docsrs, doc(cfg(feature = "async")))]
    pub fn create_async_function<'callback, A, R, F, FR>(
        &'callback self,
        func: F,
    ) -> Result<Function<'lua>>
    where
        A: FromLuaMulti<'callback>,
        R: ToLuaMulti<'callback>,
        F: 'scope + Fn(&'callback Lua, A) -> FR,
        FR: 'callback + Future<Output = Result<R>>,
    {
        unsafe {
            self.create_async_callback(Box::new(move |lua, args| {
                let args = match A::from_lua_multi(args, lua) {
                    Ok(args) => args,
                    Err(e) => return Box::pin(future::err(e)),
                };
                Box::pin(func(lua, args).and_then(move |ret| future::ready(ret.to_lua_multi(lua))))
            }))
        }
    }

    /// Create a Lua userdata object from a custom userdata type.
    ///
    /// This is a version of [`Lua::create_userdata`] that creates a userdata which expires on
    /// scope drop, and does not require that the userdata type be Send (but still requires that the
    /// UserData be 'static).
    /// See [`Lua::scope`] for more details.
    ///
    /// [`Lua::create_userdata`]: crate::Lua::create_userdata
    /// [`Lua::scope`]: crate::Lua::scope
    pub fn create_userdata<T>(&self, data: T) -> Result<AnyUserData<'lua>>
    where
        T: 'static + UserData,
    {
        self.create_userdata_inner(UserDataCell::new(data))
    }

    /// Create a Lua userdata object from a custom serializable userdata type.
    ///
    /// This is a version of [`Lua::create_ser_userdata`] that creates a userdata which expires on
    /// scope drop, and does not require that the userdata type be Send (but still requires that the
    /// UserData be 'static).
    /// See [`Lua::scope`] for more details.
    ///
    /// Requires `feature = "serialize"`
    ///
    /// [`Lua::create_ser_userdata`]: crate::Lua::create_ser_userdata
    /// [`Lua::scope`]: crate::Lua::scope
    #[cfg(feature = "serialize")]
    #[cfg_attr(docsrs, doc(cfg(feature = "serialize")))]
    pub fn create_ser_userdata<T>(&self, data: T) -> Result<AnyUserData<'lua>>
    where
        T: 'static + UserData + Serialize,
    {
        self.create_userdata_inner(UserDataCell::new_ser(data))
    }

    fn create_userdata_inner<T>(&self, data: UserDataCell<T>) -> Result<AnyUserData<'lua>>
    where
        T: 'static + UserData,
    {
        // Safe even though T may not be Send, because the parent Lua cannot be sent to another
        // thread while the Scope is alive (or the returned AnyUserData handle even).
        unsafe {
            let ud = self.lua.make_userdata(data)?;

            #[cfg(any(feature = "lua51", feature = "luajit"))]
            let newtable = self.lua.create_table()?;
            let destructor: DestructorCallback = Box::new(move |ud| {
                let state = ud.lua.state;
                let _sg = StackGuard::new(state);
                assert_stack(state, 2);

                // Check that userdata is not destructed (via `take()` call)
                if ud.lua.push_userdata_ref(&ud).is_err() {
                    return vec![];
                }

                // Clear associated user values
                #[cfg(feature = "lua54")]
                for i in 1..=USER_VALUE_MAXSLOT {
                    ffi::lua_pushnil(state);
                    ffi::lua_setiuservalue(state, -2, i as c_int);
                }
                #[cfg(any(feature = "lua53", feature = "lua52", feature = "luau"))]
                {
                    ffi::lua_pushnil(state);
                    ffi::lua_setuservalue(state, -2);
                }
                #[cfg(any(feature = "lua51", feature = "luajit"))]
                {
                    ud.lua.push_ref(&newtable.0);
                    ffi::lua_setuservalue(state, -2);
                }

                vec![Box::new(take_userdata::<UserDataCell<T>>(state))]
            });
            self.destructors
                .borrow_mut()
                .push((ud.0.clone(), destructor));

            Ok(ud)
        }
    }

    /// Create a Lua userdata object from a custom userdata type.
    ///
    /// This is a version of [`Lua::create_userdata`] that creates a userdata which expires on
    /// scope drop, and does not require that the userdata type be Send or 'static. See
    /// [`Lua::scope`] for more details.
    ///
    /// Lifting the requirement that the UserData type be 'static comes with some important
    /// limitations, so if you only need to eliminate the Send requirement, it is probably better to
    /// use [`Scope::create_userdata`] instead.
    ///
    /// The main limitation that comes from using non-'static userdata is that the produced userdata
    /// will no longer have a `TypeId` associated with it, because `TypeId` can only work for
    /// 'static types. This means that it is impossible, once the userdata is created, to get a
    /// reference to it back *out* of an `AnyUserData` handle. This also implies that the
    /// "function" type methods that can be added via [`UserDataMethods`] (the ones that accept
    /// `AnyUserData` as a first parameter) are vastly less useful. Also, there is no way to re-use
    /// a single metatable for multiple non-'static types, so there is a higher cost associated with
    /// creating the userdata metatable each time a new userdata is created.
    ///
    /// [`Scope::create_userdata`]: #method.create_userdata
    /// [`Lua::create_userdata`]: crate::Lua::create_userdata
    /// [`Lua::scope`]:crate::Lua::scope
    /// [`UserDataMethods`]: crate::UserDataMethods
    pub fn create_nonstatic_userdata<T>(&self, data: T) -> Result<AnyUserData<'lua>>
    where
        T: 'scope + UserData,
    {
        let data = Rc::new(RefCell::new(data));

        // 'callback outliving 'scope is a lie to make the types work out, required due to the
        // inability to work with the more correct callback type that is universally quantified over
        // 'lua. This is safe though, because `UserData::add_methods` does not get to pick the 'lua
        // lifetime, so none of the static methods UserData types can add can possibly capture
        // parameters.
        fn wrap_method<'scope, 'lua, 'callback: 'scope, T: 'scope>(
            scope: &Scope<'lua, 'scope>,
            data: Rc<RefCell<T>>,
            ud_ptr: *const c_void,
            method: NonStaticMethod<'callback, T>,
        ) -> Result<Function<'lua>> {
            // On methods that actually receive the userdata, we fake a type check on the passed in
            // userdata, where we pretend there is a unique type per call to
            // `Scope::create_nonstatic_userdata`. You can grab a method from a userdata and call
            // it on a mismatched userdata type, which when using normal 'static userdata will fail
            // with a type mismatch, but here without this check would proceed as though you had
            // called the method on the original value (since we otherwise completely ignore the
            // first argument).
            let check_ud_type = move |lua: &'callback Lua, value| {
                if let Some(Value::UserData(ud)) = value {
                    unsafe {
                        let _sg = StackGuard::new(lua.state);
                        check_stack(lua.state, 2)?;
                        lua.push_userdata_ref(&ud.0)?;
                        if get_userdata(lua.state, -1) as *const _ == ud_ptr {
                            return Ok(());
                        }
                    }
                };
                Err(Error::UserDataTypeMismatch)
            };

            match method {
                NonStaticMethod::Method(method) => {
                    let f = Box::new(move |lua, mut args: MultiValue<'callback>| {
                        check_ud_type(lua, args.pop_front())?;
                        let data = data.try_borrow().map_err(|_| Error::UserDataBorrowError)?;
                        method(lua, &*data, args)
                    });
                    unsafe { scope.create_callback(f) }
                }
                NonStaticMethod::MethodMut(method) => {
                    let method = RefCell::new(method);
                    let f = Box::new(move |lua, mut args: MultiValue<'callback>| {
                        check_ud_type(lua, args.pop_front())?;
                        let mut method = method
                            .try_borrow_mut()
                            .map_err(|_| Error::RecursiveMutCallback)?;
                        let mut data = data
                            .try_borrow_mut()
                            .map_err(|_| Error::UserDataBorrowMutError)?;
                        (*method)(lua, &mut *data, args)
                    });
                    unsafe { scope.create_callback(f) }
                }
                NonStaticMethod::Function(function) => unsafe { scope.create_callback(function) },
                NonStaticMethod::FunctionMut(function) => {
                    let function = RefCell::new(function);
                    let f = Box::new(move |lua, args| {
                        (*function
                            .try_borrow_mut()
                            .map_err(|_| Error::RecursiveMutCallback)?)(
                            lua, args
                        )
                    });
                    unsafe { scope.create_callback(f) }
                }
            }
        }

        let mut ud_fields = NonStaticUserDataFields::default();
        let mut ud_methods = NonStaticUserDataMethods::default();
        T::add_fields(&mut ud_fields);
        T::add_methods(&mut ud_methods);

        unsafe {
            let lua = self.lua;
            let _sg = StackGuard::new(lua.state);
            check_stack(lua.state, 13)?;

            #[cfg(not(feature = "luau"))]
            #[allow(clippy::let_and_return)]
            let ud_ptr = protect_lua!(lua.state, 0, 1, |state| {
                let ud =
                    ffi::lua_newuserdata(state, mem::size_of::<UserDataCell<Rc<RefCell<T>>>>());

                // Set empty environment for Lua 5.1
                #[cfg(any(feature = "lua51", feature = "luajit"))]
                {
                    ffi::lua_newtable(state);
                    ffi::lua_setuservalue(state, -2);
                }

                ud
            })?;
            #[cfg(feature = "luau")]
            let ud_ptr = {
                crate::util::push_userdata::<UserDataCell<Rc<RefCell<T>>>>(
                    lua.state,
                    UserDataCell::new(data.clone()),
                    true,
                )?;
                ffi::lua_touserdata(lua.state, -1)
            };

            // Prepare metatable, add meta methods first and then meta fields
            let meta_methods_nrec = ud_methods.meta_methods.len() + ud_fields.meta_fields.len() + 1;
            push_table(lua.state, 0, meta_methods_nrec as c_int, true)?;

            for (k, m) in ud_methods.meta_methods {
                let data = data.clone();
                lua.push_value(Value::Function(wrap_method(self, data, ud_ptr, m)?))?;
                rawset_field(lua.state, -2, k.validate()?.name())?;
            }
            for (k, f) in ud_fields.meta_fields {
                lua.push_value(f(mem::transmute(lua))?)?;
                rawset_field(lua.state, -2, k.validate()?.name())?;
            }
            let metatable_index = ffi::lua_absindex(lua.state, -1);

            let mut field_getters_index = None;
            let field_getters_nrec = ud_fields.field_getters.len();
            if field_getters_nrec > 0 {
                push_table(lua.state, 0, field_getters_nrec as c_int, true)?;
                for (k, m) in ud_fields.field_getters {
                    let data = data.clone();
                    lua.push_value(Value::Function(wrap_method(self, data, ud_ptr, m)?))?;
                    rawset_field(lua.state, -2, &k)?;
                }
                field_getters_index = Some(ffi::lua_absindex(lua.state, -1));
            }

            let mut field_setters_index = None;
            let field_setters_nrec = ud_fields.field_setters.len();
            if field_setters_nrec > 0 {
                push_table(lua.state, 0, field_setters_nrec as c_int, true)?;
                for (k, m) in ud_fields.field_setters {
                    let data = data.clone();
                    lua.push_value(Value::Function(wrap_method(self, data, ud_ptr, m)?))?;
                    rawset_field(lua.state, -2, &k)?;
                }
                field_setters_index = Some(ffi::lua_absindex(lua.state, -1));
            }

            let mut methods_index = None;
            let methods_nrec = ud_methods.methods.len();
            if methods_nrec > 0 {
                // Create table used for methods lookup
                push_table(lua.state, 0, methods_nrec as c_int, true)?;
                for (k, m) in ud_methods.methods {
                    let data = data.clone();
                    lua.push_value(Value::Function(wrap_method(self, data, ud_ptr, m)?))?;
                    rawset_field(lua.state, -2, &k)?;
                }
                methods_index = Some(ffi::lua_absindex(lua.state, -1));
            }

            init_userdata_metatable::<UserDataCell<Rc<RefCell<T>>>>(
                lua.state,
                metatable_index,
                field_getters_index,
                field_setters_index,
                methods_index,
            )?;

            let count = field_getters_index.map(|_| 1).unwrap_or(0)
                + field_setters_index.map(|_| 1).unwrap_or(0)
                + methods_index.map(|_| 1).unwrap_or(0);
            ffi::lua_pop(lua.state, count);

            let mt_ptr = ffi::lua_topointer(lua.state, -1);
            // Write userdata just before attaching metatable with `__gc` metamethod
            #[cfg(not(feature = "luau"))]
            std::ptr::write(ud_ptr as _, UserDataCell::new(data));
            ffi::lua_setmetatable(lua.state, -2);
            let ud = AnyUserData(lua.pop_ref());
            lua.register_userdata_metatable(mt_ptr, None);

            #[cfg(any(feature = "lua51", feature = "luajit"))]
            let newtable = lua.create_table()?;
            let destructor: DestructorCallback = Box::new(move |ud| {
                let state = ud.lua.state;
                let _sg = StackGuard::new(state);
                assert_stack(state, 2);

                // Check that userdata is valid (very likely)
                if ud.lua.push_userdata_ref(&ud).is_err() {
                    return vec![];
                }

                // Deregister metatable
                ffi::lua_getmetatable(state, -1);
                let mt_ptr = ffi::lua_topointer(state, -1);
                ffi::lua_pop(state, 1);
                ud.lua.deregister_userdata_metatable(mt_ptr);

                // Clear associated user values
                #[cfg(feature = "lua54")]
                for i in 1..=USER_VALUE_MAXSLOT {
                    ffi::lua_pushnil(state);
                    ffi::lua_setiuservalue(state, -2, i as c_int);
                }
                #[cfg(any(feature = "lua53", feature = "lua52", feature = "luau"))]
                {
                    ffi::lua_pushnil(state);
                    ffi::lua_setuservalue(state, -2);
                }
                #[cfg(any(feature = "lua51", feature = "luajit"))]
                {
                    ud.lua.push_ref(&newtable.0);
                    ffi::lua_setuservalue(state, -2);
                }

                // A hack to drop non-static `T`
                unsafe fn seal<T>(t: T) -> Box<dyn FnOnce() + 'static> {
                    let f: Box<dyn FnOnce()> = Box::new(move || drop(t));
                    mem::transmute(f)
                }

                let ud = Box::new(seal(take_userdata::<UserDataCell<Rc<RefCell<T>>>>(state)));
                vec![ud]
            });
            self.destructors
                .borrow_mut()
                .push((ud.0.clone(), destructor));

            Ok(ud)
        }
    }

    // Unsafe, because the callback can improperly capture any value with 'callback scope, such as
    // improperly capturing an argument. Since the 'callback lifetime is chosen by the user and the
    // lifetime of the callback itself is 'scope (non-'static), the borrow checker will happily pick
    // a 'callback that outlives 'scope to allow this. In order for this to be safe, the callback
    // must NOT capture any parameters.
    unsafe fn create_callback<'callback>(
        &self,
        f: Callback<'callback, 'scope>,
    ) -> Result<Function<'lua>> {
        let f = mem::transmute::<Callback<'callback, 'scope>, Callback<'lua, 'static>>(f);
        let f = self.lua.create_callback(f)?;

        let destructor: DestructorCallback = Box::new(|f| {
            let state = f.lua.state;
            let _sg = StackGuard::new(state);
            assert_stack(state, 3);

            f.lua.push_ref(&f);

            // We know the destructor has not run yet because we hold a reference to the callback.

            ffi::lua_getupvalue(state, -1, 1);
            let ud = take_userdata::<CallbackUpvalue>(state);
            ffi::lua_pushnil(state);
            ffi::lua_setupvalue(state, -2, 1);

            vec![Box::new(ud)]
        });
        self.destructors
            .borrow_mut()
            .push((f.0.clone(), destructor));

        Ok(f)
    }

    #[cfg(feature = "async")]
    unsafe fn create_async_callback<'callback>(
        &self,
        f: AsyncCallback<'callback, 'scope>,
    ) -> Result<Function<'lua>> {
        let f = mem::transmute::<AsyncCallback<'callback, 'scope>, AsyncCallback<'lua, 'static>>(f);
        let f = self.lua.create_async_callback(f)?;

        // We need to pre-allocate strings to avoid failures in destructor.
        let get_poll_str = self.lua.create_string("get_poll")?;
        let poll_str = self.lua.create_string("poll")?;
        let destructor: DestructorCallback = Box::new(move |f| {
            let state = f.lua.state;
            let _sg = StackGuard::new(state);
            assert_stack(state, 5);

            f.lua.push_ref(&f);

            // We know the destructor has not run yet because we hold a reference to the callback.

            // First, get the environment table
            #[cfg(any(feature = "lua54", feature = "lua53", feature = "lua52"))]
            ffi::lua_getupvalue(state, -1, 1);
            #[cfg(any(feature = "lua51", feature = "luajit", feature = "luau"))]
            ffi::lua_getfenv(state, -1);

            // Second, get the `get_poll()` closure using the corresponding key
            f.lua.push_ref(&get_poll_str.0);
            ffi::lua_rawget(state, -2);

            // Destroy all upvalues
            ffi::lua_getupvalue(state, -1, 1);
            let upvalue1 = take_userdata::<AsyncCallbackUpvalue>(state);
            ffi::lua_pushnil(state);
            ffi::lua_setupvalue(state, -2, 1);

            ffi::lua_pop(state, 1);
            let mut data: Vec<Box<dyn Any>> = vec![Box::new(upvalue1)];

            // Finally, get polled future and destroy it
            f.lua.push_ref(&poll_str.0);
            if ffi::lua_rawget(state, -2) == ffi::LUA_TFUNCTION {
                ffi::lua_getupvalue(state, -1, 1);
                let upvalue2 = take_userdata::<AsyncPollUpvalue>(state);
                ffi::lua_pushnil(state);
                ffi::lua_setupvalue(state, -2, 1);
                data.push(Box::new(upvalue2));
            }

            data
        });
        self.destructors
            .borrow_mut()
            .push((f.0.clone(), destructor));

        Ok(f)
    }
}

impl<'lua, 'scope> Drop for Scope<'lua, 'scope> {
    fn drop(&mut self) {
        // We separate the action of invalidating the userdata in Lua and actually dropping the
        // userdata type into two phases. This is so that, in the event a userdata drop panics, we
        // can be sure that all of the userdata in Lua is actually invalidated.

        // All destructors are non-panicking, so this is fine
        let to_drop = self
            .destructors
            .get_mut()
            .drain(..)
            .flat_map(|(r, dest)| dest(r))
            .collect::<Vec<_>>();

        drop(to_drop);
    }
}

#[allow(clippy::type_complexity)]
enum NonStaticMethod<'lua, T> {
    Method(Box<dyn Fn(&'lua Lua, &T, MultiValue<'lua>) -> Result<MultiValue<'lua>>>),
    MethodMut(Box<dyn FnMut(&'lua Lua, &mut T, MultiValue<'lua>) -> Result<MultiValue<'lua>>>),
    Function(Box<dyn Fn(&'lua Lua, MultiValue<'lua>) -> Result<MultiValue<'lua>>>),
    FunctionMut(Box<dyn FnMut(&'lua Lua, MultiValue<'lua>) -> Result<MultiValue<'lua>>>),
}

struct NonStaticUserDataMethods<'lua, T: UserData> {
    methods: Vec<(Vec<u8>, NonStaticMethod<'lua, T>)>,
    meta_methods: Vec<(MetaMethod, NonStaticMethod<'lua, T>)>,
}

impl<'lua, T: UserData> Default for NonStaticUserDataMethods<'lua, T> {
    fn default() -> NonStaticUserDataMethods<'lua, T> {
        NonStaticUserDataMethods {
            methods: Vec::new(),
            meta_methods: Vec::new(),
        }
    }
}

impl<'lua, T: UserData> UserDataMethods<'lua, T> for NonStaticUserDataMethods<'lua, T> {
    fn add_method<S, A, R, M>(&mut self, name: &S, method: M)
    where
        S: AsRef<[u8]> + ?Sized,
        A: FromLuaMulti<'lua>,
        R: ToLuaMulti<'lua>,
        M: 'static + MaybeSend + Fn(&'lua Lua, &T, A) -> Result<R>,
    {
        self.methods.push((
            name.as_ref().to_vec(),
            NonStaticMethod::Method(Box::new(move |lua, ud, args| {
                method(lua, ud, A::from_lua_multi(args, lua)?)?.to_lua_multi(lua)
            })),
        ));
    }

    fn add_method_mut<S, A, R, M>(&mut self, name: &S, mut method: M)
    where
        S: AsRef<[u8]> + ?Sized,
        A: FromLuaMulti<'lua>,
        R: ToLuaMulti<'lua>,
        M: 'static + MaybeSend + FnMut(&'lua Lua, &mut T, A) -> Result<R>,
    {
        self.methods.push((
            name.as_ref().to_vec(),
            NonStaticMethod::MethodMut(Box::new(move |lua, ud, args| {
                method(lua, ud, A::from_lua_multi(args, lua)?)?.to_lua_multi(lua)
            })),
        ));
    }

    #[cfg(feature = "async")]
    fn add_async_method<S, A, R, M, MR>(&mut self, _name: &S, _method: M)
    where
        T: Clone,
        S: AsRef<[u8]> + ?Sized,
        A: FromLuaMulti<'lua>,
        R: ToLuaMulti<'lua>,
        M: 'static + MaybeSend + Fn(&'lua Lua, T, A) -> MR,
        MR: 'lua + Future<Output = Result<R>>,
    {
        // The panic should never happen as async non-static code wouldn't compile
        // Non-static lifetime must be bounded to 'lua lifetime
        mlua_panic!("asynchronous methods are not supported for non-static userdata")
    }

    fn add_function<S, A, R, F>(&mut self, name: &S, function: F)
    where
        S: AsRef<[u8]> + ?Sized,
        A: FromLuaMulti<'lua>,
        R: ToLuaMulti<'lua>,
        F: 'static + MaybeSend + Fn(&'lua Lua, A) -> Result<R>,
    {
        self.methods.push((
            name.as_ref().to_vec(),
            NonStaticMethod::Function(Box::new(move |lua, args| {
                function(lua, A::from_lua_multi(args, lua)?)?.to_lua_multi(lua)
            })),
        ));
    }

    fn add_function_mut<S, A, R, F>(&mut self, name: &S, mut function: F)
    where
        S: AsRef<[u8]> + ?Sized,
        A: FromLuaMulti<'lua>,
        R: ToLuaMulti<'lua>,
        F: 'static + MaybeSend + FnMut(&'lua Lua, A) -> Result<R>,
    {
        self.methods.push((
            name.as_ref().to_vec(),
            NonStaticMethod::FunctionMut(Box::new(move |lua, args| {
                function(lua, A::from_lua_multi(args, lua)?)?.to_lua_multi(lua)
            })),
        ));
    }

    #[cfg(feature = "async")]
    fn add_async_function<S, A, R, F, FR>(&mut self, _name: &S, _function: F)
    where
        S: AsRef<[u8]> + ?Sized,
        A: FromLuaMulti<'lua>,
        R: ToLuaMulti<'lua>,
        F: 'static + MaybeSend + Fn(&'lua Lua, A) -> FR,
        FR: 'lua + Future<Output = Result<R>>,
    {
        // The panic should never happen as async non-static code wouldn't compile
        // Non-static lifetime must be bounded to 'lua lifetime
        mlua_panic!("asynchronous functions are not supported for non-static userdata")
    }

    fn add_meta_method<S, A, R, M>(&mut self, meta: S, method: M)
    where
        S: Into<MetaMethod>,
        A: FromLuaMulti<'lua>,
        R: ToLuaMulti<'lua>,
        M: 'static + MaybeSend + Fn(&'lua Lua, &T, A) -> Result<R>,
    {
        self.meta_methods.push((
            meta.into(),
            NonStaticMethod::Method(Box::new(move |lua, ud, args| {
                method(lua, ud, A::from_lua_multi(args, lua)?)?.to_lua_multi(lua)
            })),
        ));
    }

    fn add_meta_method_mut<S, A, R, M>(&mut self, meta: S, mut method: M)
    where
        S: Into<MetaMethod>,
        A: FromLuaMulti<'lua>,
        R: ToLuaMulti<'lua>,
        M: 'static + MaybeSend + FnMut(&'lua Lua, &mut T, A) -> Result<R>,
    {
        self.meta_methods.push((
            meta.into(),
            NonStaticMethod::MethodMut(Box::new(move |lua, ud, args| {
                method(lua, ud, A::from_lua_multi(args, lua)?)?.to_lua_multi(lua)
            })),
        ));
    }

    #[cfg(all(feature = "async", not(any(feature = "lua51", feature = "luau"))))]
    fn add_async_meta_method<S, A, R, M, MR>(&mut self, _meta: S, _method: M)
    where
        T: Clone,
        S: Into<MetaMethod>,
        A: FromLuaMulti<'lua>,
        R: ToLuaMulti<'lua>,
        M: 'static + MaybeSend + Fn(&'lua Lua, T, A) -> MR,
        MR: 'lua + Future<Output = Result<R>>,
    {
        // The panic should never happen as async non-static code wouldn't compile
        // Non-static lifetime must be bounded to 'lua lifetime
        mlua_panic!("asynchronous meta methods are not supported for non-static userdata")
    }

    fn add_meta_function<S, A, R, F>(&mut self, meta: S, function: F)
    where
        S: Into<MetaMethod>,
        A: FromLuaMulti<'lua>,
        R: ToLuaMulti<'lua>,
        F: 'static + MaybeSend + Fn(&'lua Lua, A) -> Result<R>,
    {
        self.meta_methods.push((
            meta.into(),
            NonStaticMethod::Function(Box::new(move |lua, args| {
                function(lua, A::from_lua_multi(args, lua)?)?.to_lua_multi(lua)
            })),
        ));
    }

    fn add_meta_function_mut<S, A, R, F>(&mut self, meta: S, mut function: F)
    where
        S: Into<MetaMethod>,
        A: FromLuaMulti<'lua>,
        R: ToLuaMulti<'lua>,
        F: 'static + MaybeSend + FnMut(&'lua Lua, A) -> Result<R>,
    {
        self.meta_methods.push((
            meta.into(),
            NonStaticMethod::FunctionMut(Box::new(move |lua, args| {
                function(lua, A::from_lua_multi(args, lua)?)?.to_lua_multi(lua)
            })),
        ));
    }

    #[cfg(all(feature = "async", not(any(feature = "lua51", feature = "luau"))))]
    fn add_async_meta_function<S, A, R, F, FR>(&mut self, _meta: S, _function: F)
    where
        S: Into<MetaMethod>,
        A: FromLuaMulti<'lua>,
        R: ToLuaMulti<'lua>,
        F: 'static + MaybeSend + Fn(&'lua Lua, A) -> FR,
        FR: 'lua + Future<Output = Result<R>>,
    {
        // The panic should never happen as async non-static code wouldn't compile
        // Non-static lifetime must be bounded to 'lua lifetime
        mlua_panic!("asynchronous meta functions are not supported for non-static userdata")
    }
}

struct NonStaticUserDataFields<'lua, T: UserData> {
    field_getters: Vec<(Vec<u8>, NonStaticMethod<'lua, T>)>,
    field_setters: Vec<(Vec<u8>, NonStaticMethod<'lua, T>)>,
    #[allow(clippy::type_complexity)]
    meta_fields: Vec<(MetaMethod, Box<dyn Fn(&'lua Lua) -> Result<Value<'lua>>>)>,
}

impl<'lua, T: UserData> Default for NonStaticUserDataFields<'lua, T> {
    fn default() -> NonStaticUserDataFields<'lua, T> {
        NonStaticUserDataFields {
            field_getters: Vec::new(),
            field_setters: Vec::new(),
            meta_fields: Vec::new(),
        }
    }
}

impl<'lua, T: UserData> UserDataFields<'lua, T> for NonStaticUserDataFields<'lua, T> {
    fn add_field_method_get<S, R, M>(&mut self, name: &S, method: M)
    where
        S: AsRef<[u8]> + ?Sized,
        R: ToLua<'lua>,
        M: 'static + MaybeSend + Fn(&'lua Lua, &T) -> Result<R>,
    {
        self.field_getters.push((
            name.as_ref().to_vec(),
            NonStaticMethod::Method(Box::new(move |lua, ud, _| {
                method(lua, ud)?.to_lua_multi(lua)
            })),
        ));
    }

    fn add_field_method_set<S, A, M>(&mut self, name: &S, mut method: M)
    where
        S: AsRef<[u8]> + ?Sized,
        A: FromLua<'lua>,
        M: 'static + MaybeSend + FnMut(&'lua Lua, &mut T, A) -> Result<()>,
    {
        self.field_setters.push((
            name.as_ref().to_vec(),
            NonStaticMethod::MethodMut(Box::new(move |lua, ud, args| {
                method(lua, ud, A::from_lua_multi(args, lua)?)?.to_lua_multi(lua)
            })),
        ));
    }

    fn add_field_function_get<S, R, F>(&mut self, name: &S, function: F)
    where
        S: AsRef<[u8]> + ?Sized,
        R: ToLua<'lua>,
        F: 'static + MaybeSend + Fn(&'lua Lua, AnyUserData<'lua>) -> Result<R>,
    {
        self.field_getters.push((
            name.as_ref().to_vec(),
            NonStaticMethod::Function(Box::new(move |lua, args| {
                function(lua, AnyUserData::from_lua_multi(args, lua)?)?.to_lua_multi(lua)
            })),
        ));
    }

    fn add_field_function_set<S, A, F>(&mut self, name: &S, mut function: F)
    where
        S: AsRef<[u8]> + ?Sized,
        A: FromLua<'lua>,
        F: 'static + MaybeSend + FnMut(&'lua Lua, AnyUserData<'lua>, A) -> Result<()>,
    {
        self.field_setters.push((
            name.as_ref().to_vec(),
            NonStaticMethod::FunctionMut(Box::new(move |lua, args| {
                let (ud, val) = <_>::from_lua_multi(args, lua)?;
                function(lua, ud, val)?.to_lua_multi(lua)
            })),
        ));
    }

    fn add_meta_field_with<S, R, F>(&mut self, meta: S, f: F)
    where
        S: Into<MetaMethod>,
        F: 'static + MaybeSend + Fn(&'lua Lua) -> Result<R>,
        R: ToLua<'lua>,
    {
        let meta = meta.into();
        self.meta_fields.push((
            meta.clone(),
            Box::new(move |lua| {
                let value = f(lua)?.to_lua(lua)?;
                if meta == MetaMethod::Index || meta == MetaMethod::NewIndex {
                    match value {
                        Value::Nil | Value::Table(_) | Value::Function(_) => {}
                        _ => {
                            return Err(Error::MetaMethodTypeError {
                                method: meta.to_string(),
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
}
