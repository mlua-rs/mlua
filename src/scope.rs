use std::prelude::v1::*;

use std::any::Any;
use std::cell::{Cell, RefCell};
use std::ffi::c_int;
use std::marker::PhantomData;
use std::mem;

#[cfg(feature = "serialize")]
use serde::Serialize;

use crate::error::{Error, Result};
use crate::function::Function;
use crate::lua::Lua;
use crate::types::{Callback, CallbackUpvalue, LuaRef, MaybeSend};
use crate::userdata::{
    AnyUserData, MetaMethod, UserData, UserDataCell, UserDataFields, UserDataMethods,
};
use crate::userdata_impl::UserDataRegistry;
use crate::util::{
    self, assert_stack, check_stack, init_userdata_metatable, push_string, push_table,
    rawset_field, short_type_name, take_userdata, StackGuard,
};
use crate::value::{FromLua, FromLuaMulti, IntoLua, IntoLuaMulti, Value};

#[cfg(feature = "lua54")]
use crate::userdata::USER_VALUE_MAXSLOT;

#[cfg(feature = "async")]
use std::future::Future;

/// Constructed by the [`Lua::scope`] method, allows temporarily creating Lua userdata and
/// callbacks that are not required to be Send or 'static.
///
/// See [`Lua::scope`] for more details.
///
/// [`Lua::scope`]: crate::Lua.html::scope
pub struct Scope<'lua, 'scope>
where
    'lua: 'scope,
{
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
        R: IntoLuaMulti<'callback>,
        F: Fn(&'callback Lua, A) -> Result<R> + 'scope,
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
            self.create_callback(Box::new(move |lua, nargs| {
                let args = A::from_stack_args(nargs, 1, None, lua)?;
                func(lua, args)?.push_into_stack_multi(lua)
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
        R: IntoLuaMulti<'callback>,
        F: FnMut(&'callback Lua, A) -> Result<R> + 'scope,
    {
        let func = RefCell::new(func);
        self.create_function(move |lua, args| {
            (*func
                .try_borrow_mut()
                .map_err(|_| Error::RecursiveMutCallback)?)(lua, args)
        })
    }

    /// Creates a Lua userdata object from a custom userdata type.
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
        T: UserData + 'static,
    {
        // Safe even though T may not be Send, because the parent Lua cannot be sent to another
        // thread while the Scope is alive (or the returned AnyUserData handle even).
        unsafe {
            let ud = self.lua.make_userdata(UserDataCell::new(data))?;
            self.seal_userdata::<T>(&ud)?;
            Ok(ud)
        }
    }

    /// Creates a Lua userdata object from a custom serializable userdata type.
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
        T: UserData + Serialize + 'static,
    {
        unsafe {
            let ud = self.lua.make_userdata(UserDataCell::new_ser(data))?;
            self.seal_userdata::<T>(&ud)?;
            Ok(ud)
        }
    }

    /// Creates a Lua userdata object from a reference to custom userdata type.
    ///
    /// This is a version of [`Lua::create_userdata`] that creates a userdata which expires on
    /// scope drop, and does not require that the userdata type be Send. This method takes non-'static
    /// reference to the data. See [`Lua::scope`] for more details.
    ///
    /// Userdata created with this method will not be able to be mutated from Lua.
    pub fn create_userdata_ref<T>(&self, data: &'scope T) -> Result<AnyUserData<'lua>>
    where
        T: UserData + 'static,
    {
        unsafe {
            let ud = self.lua.make_userdata(UserDataCell::new_ref(data))?;
            self.seal_userdata::<T>(&ud)?;
            Ok(ud)
        }
    }

    /// Creates a Lua userdata object from a mutable reference to custom userdata type.
    ///
    /// This is a version of [`Lua::create_userdata`] that creates a userdata which expires on
    /// scope drop, and does not require that the userdata type be Send. This method takes non-'static
    /// mutable reference to the data. See [`Lua::scope`] for more details.
    pub fn create_userdata_ref_mut<T>(&self, data: &'scope mut T) -> Result<AnyUserData<'lua>>
    where
        T: UserData + 'static,
    {
        unsafe {
            let ud = self.lua.make_userdata(UserDataCell::new_ref_mut(data))?;
            self.seal_userdata::<T>(&ud)?;
            Ok(ud)
        }
    }

    /// Creates a Lua userdata object from a custom Rust type.
    ///
    /// This is a version of [`Lua::create_any_userdata`] that creates a userdata which expires on
    /// scope drop and does not require that the userdata type be Send (but still requires that the
    /// UserData be 'static). See [`Lua::scope`] for more details.
    #[inline]
    pub fn create_any_userdata<T>(&self, data: T) -> Result<AnyUserData<'lua>>
    where
        T: 'static,
    {
        unsafe {
            let ud = self.lua.make_any_userdata(UserDataCell::new(data))?;
            self.seal_userdata::<T>(&ud)?;
            Ok(ud)
        }
    }

    /// Creates a Lua userdata object from a reference to custom Rust type.
    ///
    /// This is a version of [`Lua::create_any_userdata`] that creates a userdata which expires on
    /// scope drop, and does not require that the Rust type be Send. This method takes non-'static
    /// reference to the data. See [`Lua::scope`] for more details.
    ///
    /// Userdata created with this method will not be able to be mutated from Lua.
    pub fn create_any_userdata_ref<T>(&self, data: &'scope T) -> Result<AnyUserData<'lua>>
    where
        T: 'static,
    {
        unsafe {
            let ud = self.lua.make_any_userdata(UserDataCell::new_ref(data))?;
            self.seal_userdata::<T>(&ud)?;
            Ok(ud)
        }
    }

    /// Creates a Lua userdata object from a mutable reference to custom Rust type.
    ///
    /// This is a version of [`Lua::create_any_userdata`] that creates a userdata which expires on
    /// scope drop, and does not require that the Rust type be Send. This method takes non-'static
    /// mutable reference to the data. See [`Lua::scope`] for more details.
    pub fn create_any_userdata_ref_mut<T>(&self, data: &'scope mut T) -> Result<AnyUserData<'lua>>
    where
        T: 'static,
    {
        let lua = self.lua;
        unsafe {
            let ud = lua.make_any_userdata(UserDataCell::new_ref_mut(data))?;
            self.seal_userdata::<T>(&ud)?;
            Ok(ud)
        }
    }

    /// Shortens the lifetime of a userdata to the lifetime of the scope.
    unsafe fn seal_userdata<T: 'static>(&self, ud: &AnyUserData<'lua>) -> Result<()> {
        #[cfg(any(feature = "lua51", feature = "luajit"))]
        let newtable = self.lua.create_table()?;
        let destructor: DestructorCallback = Box::new(move |ud| {
            let state = ud.lua.state();
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
                ffi::lua_setiuservalue(state, -2, i as _);
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

        Ok(())
    }

    /// Creates a Lua userdata object from a custom userdata type.
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
        T: UserData + 'scope,
    {
        // 'callback outliving 'scope is a lie to make the types work out, required due to the
        // inability to work with the more correct callback type that is universally quantified over
        // 'lua. This is safe though, because `UserData::add_methods` does not get to pick the 'lua
        // lifetime, so none of the static methods UserData types can add can possibly capture
        // parameters.
        unsafe fn wrap_method<'scope, 'lua, 'callback: 'scope, T: 'scope>(
            scope: &Scope<'lua, 'scope>,
            ud_ptr: *const UserDataCell<T>,
            name: &str,
            method: NonStaticMethod<'callback, T>,
        ) -> Result<Function<'lua>> {
            // On methods that actually receive the userdata, we fake a type check on the passed in
            // userdata, where we pretend there is a unique type per call to
            // `Scope::create_nonstatic_userdata`. You can grab a method from a userdata and call
            // it on a mismatched userdata type, which when using normal 'static userdata will fail
            // with a type mismatch, but here without this check would proceed as though you had
            // called the method on the original value (since we otherwise completely ignore the
            // first argument).
            let func_name = format!("{}.{name}", short_type_name::<T>());
            let check_self_type = move |lua: &Lua, nargs: c_int| -> Result<&UserDataCell<T>> {
                let state = lua.state();
                if nargs > 0 && ffi::lua_touserdata(state, -nargs) as *const _ == ud_ptr {
                    return Ok(&*ud_ptr);
                }
                Err(Error::bad_self_argument(
                    &func_name,
                    Error::UserDataTypeMismatch,
                ))
            };

            match method {
                NonStaticMethod::Method(method) => {
                    let f = Box::new(move |lua, nargs| {
                        let data = check_self_type(lua, nargs)?;
                        let data = data.try_borrow()?;
                        method(lua, &*data, nargs - 1)
                    });
                    scope.create_callback(f)
                }
                NonStaticMethod::MethodMut(method) => {
                    let method = RefCell::new(method);
                    let f = Box::new(move |lua, nargs| {
                        let data = check_self_type(lua, nargs)?;
                        let mut method = method
                            .try_borrow_mut()
                            .map_err(|_| Error::RecursiveMutCallback)?;
                        let mut data = data.try_borrow_mut()?;
                        (*method)(lua, &mut *data, nargs - 1)
                    });
                    scope.create_callback(f)
                }
                NonStaticMethod::Function(function) => scope.create_callback(function),
                NonStaticMethod::FunctionMut(function) => {
                    let function = RefCell::new(function);
                    let f = Box::new(move |lua, nargs| {
                        let mut func = function
                            .try_borrow_mut()
                            .map_err(|_| Error::RecursiveMutCallback)?;
                        func(lua, nargs)
                    });
                    scope.create_callback(f)
                }
            }
        }

        let mut registry = NonStaticUserDataRegistry::new();
        T::add_fields(&mut registry);
        T::add_methods(&mut registry);

        let lua = self.lua;
        let state = lua.state();
        unsafe {
            let _sg = StackGuard::new(state);
            check_stack(state, 13)?;

            #[cfg(not(feature = "luau"))]
            let ud_ptr = protect_lua!(state, 0, 1, |state| {
                let ud = ffi::lua_newuserdata(state, mem::size_of::<UserDataCell<T>>());

                // Set empty environment for Lua 5.1
                #[cfg(any(feature = "lua51", feature = "luajit"))]
                {
                    ffi::lua_newtable(state);
                    ffi::lua_setuservalue(state, -2);
                }

                ud as *const UserDataCell<T>
            })?;
            #[cfg(feature = "luau")]
            let ud_ptr = {
                util::push_userdata(state, UserDataCell::new(data), true)?;
                ffi::lua_touserdata(state, -1) as *const UserDataCell<T>
            };

            // Prepare metatable, add meta methods first and then meta fields
            let meta_methods_nrec = registry.meta_methods.len() + registry.meta_fields.len() + 1;
            push_table(state, 0, meta_methods_nrec, true)?;
            for (k, m) in registry.meta_methods {
                lua.push_value(Value::Function(wrap_method(self, ud_ptr, &k, m)?))?;
                rawset_field(state, -2, MetaMethod::validate(&k)?)?;
            }
            let mut has_name = false;
            for (k, f) in registry.meta_fields {
                has_name = has_name || k == MetaMethod::Type;
                mlua_assert!(f(lua, 0)? == 1, "field function must return one value");
                rawset_field(state, -2, MetaMethod::validate(&k)?)?;
            }
            // Set `__name/__type` if not provided
            if !has_name {
                let type_name = short_type_name::<T>();
                push_string(state, type_name.as_bytes(), !lua.unlikely_memory_error())?;
                rawset_field(state, -2, MetaMethod::Type.name())?;
            }
            let metatable_index = ffi::lua_absindex(state, -1);

            let fields_nrec = registry.fields.len();
            if fields_nrec > 0 {
                // If __index is a table then update it inplace
                let index_type = ffi::lua_getfield(state, metatable_index, cstr!("__index"));
                match index_type {
                    ffi::LUA_TNIL | ffi::LUA_TTABLE => {
                        if index_type == ffi::LUA_TNIL {
                            // Create a new table
                            ffi::lua_pop(state, 1);
                            push_table(state, 0, fields_nrec, true)?;
                        }
                        for (k, f) in registry.fields {
                            let NonStaticMethod::Function(f) = f else {
                                unreachable!()
                            };
                            mlua_assert!(f(lua, 0)? == 1, "field function must return one value");
                            rawset_field(state, -2, &k)?;
                        }
                        rawset_field(state, metatable_index, "__index")?;
                    }
                    _ => {
                        // Propagate fields to the field getters
                        for (k, f) in registry.fields {
                            registry.field_getters.push((k, f))
                        }
                    }
                }
            }

            let mut field_getters_index = None;
            let field_getters_nrec = registry.field_getters.len();
            if field_getters_nrec > 0 {
                push_table(state, 0, field_getters_nrec, true)?;
                for (k, m) in registry.field_getters {
                    lua.push_value(Value::Function(wrap_method(self, ud_ptr, &k, m)?))?;
                    rawset_field(state, -2, &k)?;
                }
                field_getters_index = Some(ffi::lua_absindex(state, -1));
            }

            let mut field_setters_index = None;
            let field_setters_nrec = registry.field_setters.len();
            if field_setters_nrec > 0 {
                push_table(state, 0, field_setters_nrec, true)?;
                for (k, m) in registry.field_setters {
                    lua.push_value(Value::Function(wrap_method(self, ud_ptr, &k, m)?))?;
                    rawset_field(state, -2, &k)?;
                }
                field_setters_index = Some(ffi::lua_absindex(state, -1));
            }

            let mut methods_index = None;
            let methods_nrec = registry.methods.len();
            if methods_nrec > 0 {
                // Create table used for methods lookup
                push_table(state, 0, methods_nrec, true)?;
                for (k, m) in registry.methods {
                    lua.push_value(Value::Function(wrap_method(self, ud_ptr, &k, m)?))?;
                    rawset_field(state, -2, &k)?;
                }
                methods_index = Some(ffi::lua_absindex(state, -1));
            }

            #[cfg(feature = "luau")]
            let extra_init = None;
            #[cfg(not(feature = "luau"))]
            let extra_init: Option<fn(*mut ffi::lua_State) -> Result<()>> = Some(|state| {
                ffi::lua_pushcfunction(state, util::userdata_destructor::<UserDataCell<T>>);
                rawset_field(state, -2, "__gc")
            });

            init_userdata_metatable(
                state,
                metatable_index,
                field_getters_index,
                field_setters_index,
                methods_index,
                extra_init,
            )?;

            let count = field_getters_index.map(|_| 1).unwrap_or(0)
                + field_setters_index.map(|_| 1).unwrap_or(0)
                + methods_index.map(|_| 1).unwrap_or(0);
            ffi::lua_pop(state, count);

            let mt_ptr = ffi::lua_topointer(state, -1);
            // Write userdata just before attaching metatable with `__gc` metamethod
            #[cfg(not(feature = "luau"))]
            std::ptr::write(ud_ptr as _, UserDataCell::new(data));
            ffi::lua_setmetatable(state, -2);
            let ud = AnyUserData(lua.pop_ref());
            lua.register_raw_userdata_metatable(mt_ptr, None);

            #[cfg(any(feature = "lua51", feature = "luajit"))]
            let newtable = lua.create_table()?;
            let destructor: DestructorCallback = Box::new(move |ud| {
                let state = ud.lua.state();
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
                ud.lua.deregister_raw_userdata_metatable(mt_ptr);

                // Clear associated user values
                #[cfg(feature = "lua54")]
                for i in 1..=USER_VALUE_MAXSLOT {
                    ffi::lua_pushnil(state);
                    ffi::lua_setiuservalue(state, -2, i as _);
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

                let ud = take_userdata::<UserDataCell<T>>(state);
                vec![Box::new(seal(ud))]
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
            let state = f.lua.state();
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
    Method(Box<dyn Fn(&'lua Lua, &T, c_int) -> Result<c_int>>),
    MethodMut(Box<dyn FnMut(&'lua Lua, &mut T, c_int) -> Result<c_int>>),
    Function(Box<dyn Fn(&'lua Lua, c_int) -> Result<c_int>>),
    FunctionMut(Box<dyn FnMut(&'lua Lua, c_int) -> Result<c_int>>),
}

struct NonStaticUserDataRegistry<'lua, T> {
    // Fields
    fields: Vec<(String, NonStaticMethod<'lua, T>)>,
    field_getters: Vec<(String, NonStaticMethod<'lua, T>)>,
    field_setters: Vec<(String, NonStaticMethod<'lua, T>)>,
    meta_fields: Vec<(String, Callback<'lua, 'static>)>,

    // Methods
    methods: Vec<(String, NonStaticMethod<'lua, T>)>,
    meta_methods: Vec<(String, NonStaticMethod<'lua, T>)>,
}

impl<'lua, T> NonStaticUserDataRegistry<'lua, T> {
    const fn new() -> NonStaticUserDataRegistry<'lua, T> {
        NonStaticUserDataRegistry {
            fields: Vec::new(),
            field_getters: Vec::new(),
            field_setters: Vec::new(),
            meta_fields: Vec::new(),
            methods: Vec::new(),
            meta_methods: Vec::new(),
        }
    }
}

impl<'lua, T> UserDataFields<'lua, T> for NonStaticUserDataRegistry<'lua, T> {
    fn add_field<V>(&mut self, name: impl AsRef<str>, value: V)
    where
        V: IntoLua<'lua> + Clone + 'static,
    {
        let name = name.as_ref().to_string();
        self.fields.push((
            name,
            NonStaticMethod::Function(Box::new(move |lua, _| unsafe {
                value.clone().push_into_stack_multi(lua)
            })),
        ));
    }

    fn add_field_method_get<M, R>(&mut self, name: impl AsRef<str>, method: M)
    where
        M: Fn(&'lua Lua, &T) -> Result<R> + MaybeSend + 'static,
        R: IntoLua<'lua>,
    {
        let method = NonStaticMethod::Method(Box::new(move |lua, ud, _| unsafe {
            method(lua, ud)?.push_into_stack_multi(lua)
        }));
        self.field_getters.push((name.as_ref().into(), method));
    }

    fn add_field_method_set<M, A>(&mut self, name: impl AsRef<str>, mut method: M)
    where
        M: FnMut(&'lua Lua, &mut T, A) -> Result<()> + MaybeSend + 'static,
        A: FromLua<'lua>,
    {
        let func_name = format!("{}.{}", short_type_name::<T>(), name.as_ref());
        let method = NonStaticMethod::MethodMut(Box::new(move |lua, ud, nargs| unsafe {
            let val = A::from_stack_args(nargs, 2, Some(&func_name), lua)?;
            method(lua, ud, val)?.push_into_stack_multi(lua)
        }));
        self.field_setters.push((name.as_ref().into(), method));
    }

    fn add_field_function_get<F, R>(&mut self, name: impl AsRef<str>, function: F)
    where
        F: Fn(&'lua Lua, AnyUserData<'lua>) -> Result<R> + MaybeSend + 'static,
        R: IntoLua<'lua>,
    {
        let func_name = format!("{}.{}", short_type_name::<T>(), name.as_ref());
        let func = NonStaticMethod::Function(Box::new(move |lua, nargs| unsafe {
            let ud = AnyUserData::from_stack_args(nargs, 1, Some(&func_name), lua)?;
            function(lua, ud)?.push_into_stack_multi(lua)
        }));
        self.field_getters.push((name.as_ref().into(), func));
    }

    fn add_field_function_set<F, A>(&mut self, name: impl AsRef<str>, mut function: F)
    where
        F: FnMut(&'lua Lua, AnyUserData<'lua>, A) -> Result<()> + MaybeSend + 'static,
        A: FromLua<'lua>,
    {
        let func_name = format!("{}.{}", short_type_name::<T>(), name.as_ref());
        let func = NonStaticMethod::FunctionMut(Box::new(move |lua, nargs| unsafe {
            let (ud, val) = <_>::from_stack_args(nargs, 1, Some(&func_name), lua)?;
            function(lua, ud, val)?.push_into_stack_multi(lua)
        }));
        self.field_setters.push((name.as_ref().into(), func));
    }

    fn add_meta_field<V>(&mut self, name: impl AsRef<str>, value: V)
    where
        V: IntoLua<'lua> + Clone + 'static,
    {
        let name = name.as_ref().to_string();
        let name2 = name.clone();
        self.meta_fields.push((
            name,
            Box::new(move |lua, _| unsafe {
                UserDataRegistry::<()>::check_meta_field(lua, &name2, value.clone())?
                    .push_into_stack_multi(lua)
            }),
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
            Box::new(move |lua, _| unsafe {
                UserDataRegistry::<()>::check_meta_field(lua, &name2, f(lua)?)?
                    .push_into_stack_multi(lua)
            }),
        ));
    }
}

impl<'lua, T> UserDataMethods<'lua, T> for NonStaticUserDataRegistry<'lua, T> {
    fn add_method<M, A, R>(&mut self, name: impl AsRef<str>, method: M)
    where
        M: Fn(&'lua Lua, &T, A) -> Result<R> + MaybeSend + 'static,
        A: FromLuaMulti<'lua>,
        R: IntoLuaMulti<'lua>,
    {
        let func_name = format!("{}.{}", short_type_name::<T>(), name.as_ref());
        let method = NonStaticMethod::Method(Box::new(move |lua, ud, nargs| unsafe {
            let args = A::from_stack_args(nargs, 2, Some(&func_name), lua)?;
            method(lua, ud, args)?.push_into_stack_multi(lua)
        }));
        self.methods.push((name.as_ref().into(), method));
    }

    fn add_method_mut<M, A, R>(&mut self, name: impl AsRef<str>, mut method: M)
    where
        M: FnMut(&'lua Lua, &mut T, A) -> Result<R> + MaybeSend + 'static,
        A: FromLuaMulti<'lua>,
        R: IntoLuaMulti<'lua>,
    {
        let func_name = format!("{}.{}", short_type_name::<T>(), name.as_ref());
        let method = NonStaticMethod::MethodMut(Box::new(move |lua, ud, nargs| unsafe {
            let args = A::from_stack_args(nargs, 2, Some(&func_name), lua)?;
            method(lua, ud, args)?.push_into_stack_multi(lua)
        }));
        self.methods.push((name.as_ref().into(), method));
    }

    #[cfg(feature = "async")]
    fn add_async_method<'s, M, A, MR, R>(&mut self, _name: impl AsRef<str>, _method: M)
    where
        'lua: 's,
        T: 'static,
        M: Fn(&'lua Lua, &'s T, A) -> MR + MaybeSend + 'static,
        A: FromLuaMulti<'lua>,
        MR: Future<Output = Result<R>> + 's,
        R: IntoLuaMulti<'lua>,
    {
        // The panic should never happen as async non-static code wouldn't compile
        // Non-static lifetime must be bounded to 'lua lifetime
        panic!("asynchronous methods are not supported for non-static userdata")
    }

    #[cfg(feature = "async")]
    fn add_async_method_mut<'s, M, A, MR, R>(&mut self, _name: impl AsRef<str>, _method: M)
    where
        'lua: 's,
        T: 'static,
        M: Fn(&'lua Lua, &'s mut T, A) -> MR + MaybeSend + 'static,
        A: FromLuaMulti<'lua>,
        MR: Future<Output = Result<R>> + 's,
        R: IntoLuaMulti<'lua>,
    {
        // The panic should never happen as async non-static code wouldn't compile
        // Non-static lifetime must be bounded to 'lua lifetime
        panic!("asynchronous methods are not supported for non-static userdata")
    }

    fn add_function<F, A, R>(&mut self, name: impl AsRef<str>, function: F)
    where
        F: Fn(&'lua Lua, A) -> Result<R> + MaybeSend + 'static,
        A: FromLuaMulti<'lua>,
        R: IntoLuaMulti<'lua>,
    {
        let func_name = format!("{}.{}", short_type_name::<T>(), name.as_ref());
        let func = NonStaticMethod::Function(Box::new(move |lua, nargs| unsafe {
            let args = A::from_stack_args(nargs, 1, Some(&func_name), lua)?;
            function(lua, args)?.push_into_stack_multi(lua)
        }));
        self.methods.push((name.as_ref().into(), func));
    }

    fn add_function_mut<F, A, R>(&mut self, name: impl AsRef<str>, mut function: F)
    where
        F: FnMut(&'lua Lua, A) -> Result<R> + MaybeSend + 'static,
        A: FromLuaMulti<'lua>,
        R: IntoLuaMulti<'lua>,
    {
        let func_name = format!("{}.{}", short_type_name::<T>(), name.as_ref());
        let func = NonStaticMethod::FunctionMut(Box::new(move |lua, nargs| unsafe {
            let args = A::from_stack_args(nargs, 1, Some(&func_name), lua)?;
            function(lua, args)?.push_into_stack_multi(lua)
        }));
        self.methods.push((name.as_ref().into(), func));
    }

    #[cfg(feature = "async")]
    fn add_async_function<F, A, FR, R>(&mut self, _name: impl AsRef<str>, _function: F)
    where
        F: Fn(&'lua Lua, A) -> FR + MaybeSend + 'static,
        A: FromLuaMulti<'lua>,
        FR: Future<Output = Result<R>> + 'lua,
        R: IntoLuaMulti<'lua>,
    {
        // The panic should never happen as async non-static code wouldn't compile
        // Non-static lifetime must be bounded to 'lua lifetime
        panic!("asynchronous functions are not supported for non-static userdata")
    }

    fn add_meta_method<M, A, R>(&mut self, name: impl AsRef<str>, method: M)
    where
        M: Fn(&'lua Lua, &T, A) -> Result<R> + MaybeSend + 'static,
        A: FromLuaMulti<'lua>,
        R: IntoLuaMulti<'lua>,
    {
        let func_name = format!("{}.{}", short_type_name::<T>(), name.as_ref());
        let method = NonStaticMethod::Method(Box::new(move |lua, ud, nargs| unsafe {
            let args = A::from_stack_args(nargs, 2, Some(&func_name), lua)?;
            method(lua, ud, args)?.push_into_stack_multi(lua)
        }));
        self.meta_methods.push((name.as_ref().into(), method));
    }

    fn add_meta_method_mut<M, A, R>(&mut self, name: impl AsRef<str>, mut method: M)
    where
        M: FnMut(&'lua Lua, &mut T, A) -> Result<R> + MaybeSend + 'static,
        A: FromLuaMulti<'lua>,
        R: IntoLuaMulti<'lua>,
    {
        let func_name = format!("{}.{}", short_type_name::<T>(), name.as_ref());
        let method = NonStaticMethod::MethodMut(Box::new(move |lua, ud, nargs| unsafe {
            let args = A::from_stack_args(nargs, 2, Some(&func_name), lua)?;
            method(lua, ud, args)?.push_into_stack_multi(lua)
        }));
        self.meta_methods.push((name.as_ref().into(), method));
    }

    #[cfg(all(feature = "async", not(any(feature = "lua51", feature = "luau"))))]
    fn add_async_meta_method<'s, M, A, MR, R>(&mut self, _name: impl AsRef<str>, _method: M)
    where
        'lua: 's,
        T: 'static,
        M: Fn(&'lua Lua, &'s T, A) -> MR + MaybeSend + 'static,
        A: FromLuaMulti<'lua>,
        MR: Future<Output = Result<R>> + 's,
        R: IntoLuaMulti<'lua>,
    {
        // The panic should never happen as async non-static code wouldn't compile
        // Non-static lifetime must be bounded to 'lua lifetime
        panic!("asynchronous meta methods are not supported for non-static userdata")
    }

    #[cfg(all(feature = "async", not(any(feature = "lua51", feature = "luau"))))]
    fn add_async_meta_method_mut<'s, M, A, MR, R>(&mut self, _name: impl AsRef<str>, _method: M)
    where
        'lua: 's,
        T: 'static,
        M: Fn(&'lua Lua, &'s mut T, A) -> MR + MaybeSend + 'static,
        A: FromLuaMulti<'lua>,
        MR: Future<Output = Result<R>> + 's,
        R: IntoLuaMulti<'lua>,
    {
        // The panic should never happen as async non-static code wouldn't compile
        // Non-static lifetime must be bounded to 'lua lifetime
        panic!("asynchronous meta methods are not supported for non-static userdata")
    }

    fn add_meta_function<F, A, R>(&mut self, name: impl AsRef<str>, function: F)
    where
        F: Fn(&'lua Lua, A) -> Result<R> + MaybeSend + 'static,
        A: FromLuaMulti<'lua>,
        R: IntoLuaMulti<'lua>,
    {
        let func_name = format!("{}.{}", short_type_name::<T>(), name.as_ref());
        let func = NonStaticMethod::Function(Box::new(move |lua, nargs| unsafe {
            let args = A::from_stack_args(nargs, 1, Some(&func_name), lua)?;
            function(lua, args)?.push_into_stack_multi(lua)
        }));
        self.meta_methods.push((name.as_ref().into(), func));
    }

    fn add_meta_function_mut<F, A, R>(&mut self, name: impl AsRef<str>, mut function: F)
    where
        F: FnMut(&'lua Lua, A) -> Result<R> + MaybeSend + 'static,
        A: FromLuaMulti<'lua>,
        R: IntoLuaMulti<'lua>,
    {
        let func_name = format!("{}.{}", short_type_name::<T>(), name.as_ref());
        let func = NonStaticMethod::FunctionMut(Box::new(move |lua, nargs| unsafe {
            let args = A::from_stack_args(nargs, 1, Some(&func_name), lua)?;
            function(lua, args)?.push_into_stack_multi(lua)
        }));
        self.meta_methods.push((name.as_ref().into(), func));
    }

    #[cfg(all(feature = "async", not(any(feature = "lua51", feature = "luau"))))]
    fn add_async_meta_function<F, A, FR, R>(&mut self, _name: impl AsRef<str>, _function: F)
    where
        F: Fn(&'lua Lua, A) -> FR + MaybeSend + 'static,
        A: FromLuaMulti<'lua>,
        FR: Future<Output = Result<R>> + 'lua,
        R: IntoLuaMulti<'lua>,
    {
        // The panic should never happen as async non-static code wouldn't compile
        // Non-static lifetime must be bounded to 'lua lifetime
        panic!("asynchronous meta functions are not supported for non-static userdata")
    }
}
