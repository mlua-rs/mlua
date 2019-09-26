use std::any::TypeId;
use std::cell::{RefCell, UnsafeCell};
use std::collections::HashMap;
use std::ffi::CString;
use std::marker::PhantomData;
use std::os::raw::{c_char, c_int, c_void};
use std::string::String as StdString;
use std::sync::{Arc, Mutex};
use std::{mem, ptr, str};

use libc;

use error::{Error, Result};
use ffi;
use function::Function;
use scope::Scope;
use string::String;
use table::Table;
use thread::Thread;
use types::{Callback, Integer, LightUserData, LuaRef, Number, RegistryKey};
use userdata::{AnyUserData, MetaMethod, UserData, UserDataMethods};
use util::{
    assert_stack, callback_error, check_stack, gc_guard, get_userdata, get_wrapped_error,
    init_error_metatables, init_userdata_metatable, main_state, pop_error, protect_lua,
    protect_lua_closure, push_string, push_userdata, push_wrapped_error, safe_pcall, safe_xpcall,
    userdata_destructor, StackGuard,
};
use value::{FromLua, FromLuaMulti, MultiValue, Nil, ToLua, ToLuaMulti, Value};

/// Top level Lua struct which holds the Lua state itself.
pub struct Lua {
    pub(crate) state: *mut ffi::lua_State,
    main_state: *mut ffi::lua_State,
    // Lua has lots of interior mutability, should not be RefUnwindSafe
    _phantom: PhantomData<UnsafeCell<()>>,
}

unsafe impl Send for Lua {}

impl Lua {
    /// Constructs a new Lua instance from the existing state.
    pub unsafe fn init_from_ptr(state: *mut ffi::lua_State) -> Lua {
        let state_top = ffi::lua_gettop(state);

        init_error_metatables(state);

        // Create the function metatable

        ffi::lua_pushlightuserdata(
            state,
            &FUNCTION_METATABLE_REGISTRY_KEY as *const u8 as *mut c_void,
        );

        ffi::lua_newtable(state);

        push_string(state, "__gc").unwrap();
        ffi::lua_pushcfunction(state, userdata_destructor::<Callback>);
        ffi::lua_rawset(state, -3);

        push_string(state, "__metatable").unwrap();
        ffi::lua_pushboolean(state, 0);
        ffi::lua_rawset(state, -3);

        ffi::lua_rawset(state, ffi::LUA_REGISTRYINDEX);

        // Create ref stack thread and place it in the registry to prevent it from being garbage
        // collected.

        let ref_thread = ffi::lua_newthread(state);
        ffi::luaL_ref(state, ffi::LUA_REGISTRYINDEX);

        // Create ExtraData, and place it in the lua_State "extra space"

        let extra = Box::into_raw(Box::new(ExtraData {
            registered_userdata: HashMap::new(),
            registry_unref_list: Arc::new(Mutex::new(Some(Vec::new()))),
            ref_thread,
            // We need 1 extra stack space to move values in and out of the ref stack.
            ref_stack_size: ffi::LUA_MINSTACK - 1,
            ref_stack_max: 0,
            ref_free: Vec::new(),
        }));
        *(ffi::lua_getextraspace(state) as *mut *mut ExtraData) = extra;

        rlua_debug_assert!(ffi::lua_gettop(state) == state_top, "stack leak during creation");
        assert_stack(state, ffi::LUA_MINSTACK);

        Lua {
            state,
            main_state: main_state(state),
            _phantom: PhantomData,
        }
    }

    /// Loads a chunk of Lua code and returns it as a function.
    ///
    /// The source can be named by setting the `name` parameter. This is generally recommended as it
    /// results in better error traces.
    ///
    /// Equivalent to Lua's `load` function.
    pub fn load<S>(&self, source: &S, name: Option<&str>) -> Result<Function>
    where
        S: ?Sized + AsRef<[u8]>,
    {
        unsafe {
            let _sg = StackGuard::new(self.state);
            assert_stack(self.state, 1);
            let source = source.as_ref();

            match if let Some(name) = name {
                let name =
                    CString::new(name.to_owned()).map_err(|e| Error::ToLuaConversionError {
                        from: "&str",
                        to: "string",
                        message: Some(e.to_string()),
                    })?;
                ffi::luaL_loadbuffer(
                    self.state,
                    source.as_ptr() as *const c_char,
                    source.len(),
                    name.as_ptr(),
                )
            } else {
                ffi::luaL_loadbuffer(
                    self.state,
                    source.as_ptr() as *const c_char,
                    source.len(),
                    ptr::null(),
                )
            } {
                ffi::LUA_OK => Ok(Function(self.pop_ref())),
                err => Err(pop_error(self.state, err)),
            }
        }
    }

    /// Execute a chunk of Lua code.
    ///
    /// This is equivalent to simply loading the source with `load` and then calling the resulting
    /// function with no arguments.
    ///
    /// Returns the values returned by the chunk.
    pub fn exec<'lua, S, R: FromLuaMulti<'lua>>(
        &'lua self,
        source: &S,
        name: Option<&str>,
    ) -> Result<R>
    where
        S: ?Sized + AsRef<[u8]>,
        R: FromLuaMulti<'lua>,
    {
        self.load(source, name)?.call(())
    }

    /// Evaluate the given expression or chunk inside this Lua state.
    ///
    /// If `source` is an expression, returns the value it evaluates to. Otherwise, returns the
    /// values returned by the chunk (if any).
    pub fn eval<'lua, S, R>(&'lua self, source: &S, name: Option<&str>) -> Result<R>
    where
        S: ?Sized + AsRef<[u8]>,
        R: FromLuaMulti<'lua>,
    {
        // First, try interpreting the lua as an expression by adding
        // "return", then as a statement.  This is the same thing the
        // actual lua repl does.
        let mut return_source = "return ".as_bytes().to_vec();
        return_source.extend(source.as_ref());
        self.load(&return_source, name)
            .or_else(|_| self.load(source, name))?
            .call(())
    }

    /// Create and return an interned Lua string.  Lua strings can be arbitrary [u8] data including
    /// embedded nulls, so in addition to `&str` and `&String`, you can also pass plain `&[u8]`
    /// here.
    pub fn create_string<S>(&self, s: &S) -> Result<String>
    where
        S: ?Sized + AsRef<[u8]>,
    {
        unsafe {
            let _sg = StackGuard::new(self.state);
            assert_stack(self.state, 4);
            push_string(self.state, s)?;
            Ok(String(self.pop_ref()))
        }
    }

    /// Creates and returns a new table.
    pub fn create_table(&self) -> Result<Table> {
        unsafe {
            let _sg = StackGuard::new(self.state);
            assert_stack(self.state, 3);
            unsafe extern "C" fn new_table(state: *mut ffi::lua_State) -> c_int {
                ffi::lua_newtable(state);
                1
            }
            protect_lua(self.state, 0, new_table)?;
            Ok(Table(self.pop_ref()))
        }
    }

    /// Creates a table and fills it with values from an iterator.
    pub fn create_table_from<'lua, K, V, I>(&'lua self, cont: I) -> Result<Table<'lua>>
    where
        K: ToLua<'lua>,
        V: ToLua<'lua>,
        I: IntoIterator<Item = (K, V)>,
    {
        unsafe {
            let _sg = StackGuard::new(self.state);
            // `Lua` instance assumes that on any callback, the Lua stack has at least LUA_MINSTACK
            // slots available to avoid panics.
            check_stack(self.state, 5 + ffi::LUA_MINSTACK)?;

            unsafe extern "C" fn new_table(state: *mut ffi::lua_State) -> c_int {
                ffi::lua_newtable(state);
                1
            }
            protect_lua(self.state, 0, new_table)?;

            for (k, v) in cont {
                self.push_value(k.to_lua(self)?);
                self.push_value(v.to_lua(self)?);
                unsafe extern "C" fn raw_set(state: *mut ffi::lua_State) -> c_int {
                    ffi::lua_rawset(state, -3);
                    1
                }
                protect_lua(self.state, 3, raw_set)?;
            }
            Ok(Table(self.pop_ref()))
        }
    }

    /// Creates a table from an iterator of values, using `1..` as the keys.
    pub fn create_sequence_from<'lua, T, I>(&'lua self, cont: I) -> Result<Table<'lua>>
    where
        T: ToLua<'lua>,
        I: IntoIterator<Item = T>,
    {
        self.create_table_from(cont.into_iter().enumerate().map(|(k, v)| (k + 1, v)))
    }

    /// Wraps a Rust function or closure, creating a callable Lua function handle to it.
    ///
    /// The function's return value is always a `Result`: If the function returns `Err`, the error
    /// is raised as a Lua error, which can be caught using `(x)pcall` or bubble up to the Rust code
    /// that invoked the Lua code. This allows using the `?` operator to propagate errors through
    /// intermediate Lua code.
    ///
    /// If the function returns `Ok`, the contained value will be converted to one or more Lua
    /// values. For details on Rust-to-Lua conversions, refer to the [`ToLua`] and [`ToLuaMulti`]
    /// traits.
    ///
    /// # Examples
    ///
    /// Create a function which prints its argument:
    ///
    /// ```
    /// # extern crate rlua;
    /// # use rlua::{Lua, Result};
    /// # fn try_main() -> Result<()> {
    /// let lua = Lua::new();
    ///
    /// let greet = lua.create_function(|_, name: String| {
    ///     println!("Hello, {}!", name);
    ///     Ok(())
    /// });
    /// # let _ = greet;    // used
    /// # Ok(())
    /// # }
    /// # fn main() {
    /// #     try_main().unwrap();
    /// # }
    /// ```
    ///
    /// Use tuples to accept multiple arguments:
    ///
    /// ```
    /// # extern crate rlua;
    /// # use rlua::{Lua, Result};
    /// # fn try_main() -> Result<()> {
    /// let lua = Lua::new();
    ///
    /// let print_person = lua.create_function(|_, (name, age): (String, u8)| {
    ///     println!("{} is {} years old!", name, age);
    ///     Ok(())
    /// });
    /// # let _ = print_person;    // used
    /// # Ok(())
    /// # }
    /// # fn main() {
    /// #     try_main().unwrap();
    /// # }
    /// ```
    ///
    /// [`ToLua`]: trait.ToLua.html
    /// [`ToLuaMulti`]: trait.ToLuaMulti.html
    pub fn create_function<'lua, 'callback, A, R, F>(&'lua self, func: F) -> Result<Function<'lua>>
    where
        A: FromLuaMulti<'callback>,
        R: ToLuaMulti<'callback>,
        F: 'static + Send + Fn(&'callback Lua, A) -> Result<R>,
    {
        self.create_callback(Box::new(move |lua, args| {
            func(lua, A::from_lua_multi(args, lua)?)?.to_lua_multi(lua)
        }))
    }

    /// Wraps a Rust mutable closure, creating a callable Lua function handle to it.
    ///
    /// This is a version of [`create_function`] that accepts a FnMut argument.  Refer to
    /// [`create_function`] for more information about the implementation.
    ///
    /// [`create_function`]: #method.create_function
    pub fn create_function_mut<'lua, 'callback, A, R, F>(
        &'lua self,
        func: F,
    ) -> Result<Function<'lua>>
    where
        A: FromLuaMulti<'callback>,
        R: ToLuaMulti<'callback>,
        F: 'static + Send + FnMut(&'callback Lua, A) -> Result<R>,
    {
        let func = RefCell::new(func);
        self.create_function(move |lua, args| {
            (&mut *func
                .try_borrow_mut()
                .map_err(|_| Error::RecursiveMutCallback)?)(lua, args)
        })
    }

    /// Wraps a Lua function into a new thread (or coroutine).
    ///
    /// Equivalent to `coroutine.create`.
    pub fn create_thread<'lua>(&'lua self, func: Function<'lua>) -> Result<Thread<'lua>> {
        unsafe {
            let _sg = StackGuard::new(self.state);
            assert_stack(self.state, 2);

            let thread_state =
                protect_lua_closure(self.state, 0, 1, |state| ffi::lua_newthread(state))?;
            self.push_ref(&func.0);
            ffi::lua_xmove(self.state, thread_state, 1);

            Ok(Thread(self.pop_ref()))
        }
    }

    /// Create a Lua userdata object from a custom userdata type.
    pub fn create_userdata<T>(&self, data: T) -> Result<AnyUserData>
    where
        T: 'static + Send + UserData,
    {
        unsafe { self.make_userdata(data) }
    }

    /// Returns a handle to the global environment.
    pub fn globals(&self) -> Table {
        unsafe {
            let _sg = StackGuard::new(self.state);
            assert_stack(self.state, 2);
            ffi::lua_rawgeti(self.state, ffi::LUA_REGISTRYINDEX, ffi::LUA_RIDX_GLOBALS);
            Table(self.pop_ref())
        }
    }

    /// Calls the given function with a `Scope` parameter, giving the function the ability to create
    /// userdata and callbacks from rust types that are !Send or non-'static.
    ///
    /// The lifetime of any function or userdata created through `Scope` lasts only until the
    /// completion of this method call, on completion all such created values are automatically
    /// dropped and Lua references to them are invalidated.  If a script accesses a value created
    /// through `Scope` outside of this method, a Lua error will result.  Since we can ensure the
    /// lifetime of values created through `Scope`, and we know that `Lua` cannot be sent to another
    /// thread while `Scope` is live, it is safe to allow !Send datatypes and whose lifetimes only
    /// outlive the scope lifetime.
    ///
    /// Handles that `Lua::scope` produces have a `'lua` lifetime of the scope parameter, to prevent
    /// the handles from escaping the callback.  However, this is not the only way for values to
    /// escape the callback, as they can be smuggled through Lua itself.  This is safe to do, but
    /// not very useful, because after the scope is dropped, all references to scoped values,
    /// whether in Lua or in rust, are invalidated.  `Function` types will error when called, and
    /// `AnyUserData` types will be typeless.
    pub fn scope<'scope, 'lua: 'scope, F, R>(&'lua self, f: F) -> R
    where
        F: FnOnce(&Scope<'scope>) -> R,
    {
        let scope = Scope::new(self);
        let r = f(&scope);
        drop(scope);
        r
    }

    /// Attempts to coerce a Lua value into a String in a manner consistent with Lua's internal
    /// behavior.
    ///
    /// To succeed, the value must be a string (in which case this is a no-op), an integer, or a
    /// number.
    pub fn coerce_string<'lua>(&'lua self, v: Value<'lua>) -> Option<String<'lua>> {
        match v {
            Value::String(s) => Some(s),
            v => unsafe {
                let _sg = StackGuard::new(self.state);
                assert_stack(self.state, 4);

                self.push_value(v);
                let s = gc_guard(self.state, || ffi::lua_tostring(self.state, -1));
                if s.is_null() {
                    None
                } else {
                    Some(String(self.pop_ref()))
                }
            },
        }
    }

    /// Attempts to coerce a Lua value into an integer in a manner consistent with Lua's internal
    /// behavior.
    ///
    /// To succeed, the value must be an integer, a floating point number that has an exact
    /// representation as an integer, or a string that can be converted to an integer. Refer to the
    /// Lua manual for details.
    pub fn coerce_integer(&self, v: Value) -> Option<Integer> {
        match v {
            Value::Integer(i) => Some(i),
            v => unsafe {
                let _sg = StackGuard::new(self.state);
                assert_stack(self.state, 2);

                self.push_value(v);
                let mut isint = 0;
                let i = ffi::lua_tointegerx(self.state, -1, &mut isint);
                if isint == 0 {
                    None
                } else {
                    Some(i)
                }
            },
        }
    }

    /// Attempts to coerce a Lua value into a Number in a manner consistent with Lua's internal
    /// behavior.
    ///
    /// To succeed, the value must be a number or a string that can be converted to a number. Refer
    /// to the Lua manual for details.
    pub fn coerce_number(&self, v: Value) -> Option<Number> {
        match v {
            Value::Number(n) => Some(n),
            v => unsafe {
                let _sg = StackGuard::new(self.state);
                assert_stack(self.state, 2);

                self.push_value(v);
                let mut isnum = 0;
                let n = ffi::lua_tonumberx(self.state, -1, &mut isnum);
                if isnum == 0 {
                    None
                } else {
                    Some(n)
                }
            },
        }
    }

    /// Converts a value that implements `ToLua` into a `Value` instance.
    pub fn pack<'lua, T: ToLua<'lua>>(&'lua self, t: T) -> Result<Value<'lua>> {
        t.to_lua(self)
    }

    /// Converts a `Value` instance into a value that implements `FromLua`.
    pub fn unpack<'lua, T: FromLua<'lua>>(&'lua self, value: Value<'lua>) -> Result<T> {
        T::from_lua(value, self)
    }

    /// Converts a value that implements `ToLuaMulti` into a `MultiValue` instance.
    pub fn pack_multi<'lua, T: ToLuaMulti<'lua>>(&'lua self, t: T) -> Result<MultiValue<'lua>> {
        t.to_lua_multi(self)
    }

    /// Converts a `MultiValue` instance into a value that implements `FromLuaMulti`.
    pub fn unpack_multi<'lua, T: FromLuaMulti<'lua>>(
        &'lua self,
        value: MultiValue<'lua>,
    ) -> Result<T> {
        T::from_lua_multi(value, self)
    }

    /// Set a value in the Lua registry based on a string name.
    ///
    /// This value will be available to rust from all `Lua` instances which share the same main
    /// state.
    pub fn set_named_registry_value<'lua, T: ToLua<'lua>>(
        &'lua self,
        name: &str,
        t: T,
    ) -> Result<()> {
        let t = t.to_lua(self)?;
        unsafe {
            let _sg = StackGuard::new(self.state);
            assert_stack(self.state, 5);

            push_string(self.state, name)?;
            self.push_value(t);

            unsafe extern "C" fn set_registry(state: *mut ffi::lua_State) -> c_int {
                ffi::lua_rawset(state, ffi::LUA_REGISTRYINDEX);
                0
            }
            protect_lua(self.state, 2, set_registry)
        }
    }

    /// Get a value from the Lua registry based on a string name.
    ///
    /// Any Lua instance which shares the underlying main state may call this method to
    /// get a value previously set by [`set_named_registry_value`].
    ///
    /// [`set_named_registry_value`]: #method.set_named_registry_value
    pub fn named_registry_value<'lua, T: FromLua<'lua>>(&'lua self, name: &str) -> Result<T> {
        let value = unsafe {
            let _sg = StackGuard::new(self.state);
            assert_stack(self.state, 4);

            push_string(self.state, name)?;
            unsafe extern "C" fn get_registry(state: *mut ffi::lua_State) -> c_int {
                ffi::lua_rawget(state, ffi::LUA_REGISTRYINDEX);
                1
            }
            protect_lua(self.state, 1, get_registry)?;

            self.pop_value()
        };
        T::from_lua(value, self)
    }

    /// Removes a named value in the Lua registry.
    ///
    /// Equivalent to calling [`set_named_registry_value`] with a value of Nil.
    ///
    /// [`set_named_registry_value`]: #method.set_named_registry_value
    pub fn unset_named_registry_value<'lua>(&'lua self, name: &str) -> Result<()> {
        self.set_named_registry_value(name, Nil)
    }

    /// Place a value in the Lua registry with an auto-generated key.
    ///
    /// This value will be available to rust from all `Lua` instances which share the same main
    /// state.
    pub fn create_registry_value<'lua, T: ToLua<'lua>>(&'lua self, t: T) -> Result<RegistryKey> {
        let t = t.to_lua(self)?;
        unsafe {
            let _sg = StackGuard::new(self.state);
            assert_stack(self.state, 2);

            self.push_value(t);
            let registry_id = gc_guard(self.state, || {
                ffi::luaL_ref(self.state, ffi::LUA_REGISTRYINDEX)
            });

            Ok(RegistryKey {
                registry_id,
                unref_list: (*extra_data(self.state)).registry_unref_list.clone(),
            })
        }
    }

    /// Get a value from the Lua registry by its `RegistryKey`
    ///
    /// Any Lua instance which shares the underlying main state may call this method to get a value
    /// previously placed by [`create_registry_value`].
    ///
    /// [`create_registry_value`]: #method.create_registry_value
    pub fn registry_value<'lua, T: FromLua<'lua>>(&'lua self, key: &RegistryKey) -> Result<T> {
        let value = unsafe {
            if !self.owns_registry_value(key) {
                return Err(Error::MismatchedRegistryKey);
            }

            let _sg = StackGuard::new(self.state);
            assert_stack(self.state, 2);

            ffi::lua_rawgeti(
                self.state,
                ffi::LUA_REGISTRYINDEX,
                key.registry_id as ffi::lua_Integer,
            );
            self.pop_value()
        };
        T::from_lua(value, self)
    }

    /// Removes a value from the Lua registry.
    ///
    /// You may call this function to manually remove a value placed in the registry with
    /// [`create_registry_value`]. In addition to manual `RegistryKey` removal, you can also call
    /// [`expire_registry_values`] to automatically remove values from the registry whose
    /// `RegistryKey`s have been dropped.
    ///
    /// [`create_registry_value`]: #method.create_registry_value
    /// [`expire_registry_values`]: #method.expire_registry_values
    pub fn remove_registry_value(&self, key: RegistryKey) -> Result<()> {
        unsafe {
            if !self.owns_registry_value(&key) {
                return Err(Error::MismatchedRegistryKey);
            }

            ffi::luaL_unref(self.state, ffi::LUA_REGISTRYINDEX, key.take());
            Ok(())
        }
    }

    /// Returns true if the given `RegistryKey` was created by a `Lua` which shares the underlying
    /// main state with this `Lua` instance.
    ///
    /// Other than this, methods that accept a `RegistryKey` will return
    /// `Error::MismatchedRegistryKey` if passed a `RegistryKey` that was not created with a
    /// matching `Lua` state.
    pub fn owns_registry_value(&self, key: &RegistryKey) -> bool {
        unsafe {
            Arc::ptr_eq(
                &key.unref_list,
                &(*extra_data(self.state)).registry_unref_list,
            )
        }
    }

    /// Remove any registry values whose `RegistryKey`s have all been dropped.
    ///
    /// Unlike normal handle values, `RegistryKey`s do not automatically remove themselves on Drop,
    /// but you can call this method to remove any unreachable registry values not manually removed
    /// by `Lua::remove_registry_value`.
    pub fn expire_registry_values(&self) {
        unsafe {
            let unref_list = mem::replace(
                &mut *(*extra_data(self.state))
                    .registry_unref_list
                    .lock()
                    .unwrap(),
                Some(Vec::new()),
            );
            for id in unref_list.unwrap() {
                ffi::luaL_unref(self.state, ffi::LUA_REGISTRYINDEX, id);
            }
        }
    }

    // Uses 2 stack spaces, does not call checkstack
    // TODO: return to original
    pub unsafe fn push_value(&self, value: Value) {
        match value {
            Value::Nil => {
                ffi::lua_pushnil(self.state);
            }

            Value::Boolean(b) => {
                ffi::lua_pushboolean(self.state, if b { 1 } else { 0 });
            }

            Value::LightUserData(ud) => {
                ffi::lua_pushlightuserdata(self.state, ud.0);
            }

            Value::Integer(i) => {
                ffi::lua_pushinteger(self.state, i);
            }

            Value::Number(n) => {
                ffi::lua_pushnumber(self.state, n);
            }

            Value::String(s) => {
                self.push_ref(&s.0);
            }

            Value::Table(t) => {
                self.push_ref(&t.0);
            }

            Value::Function(f) => {
                self.push_ref(&f.0);
            }

            Value::Thread(t) => {
                self.push_ref(&t.0);
            }

            Value::UserData(ud) => {
                self.push_ref(&ud.0);
            }

            Value::Error(e) => {
                push_wrapped_error(self.state, e);
            }
        }
    }

    // Uses 2 stack spaces, does not call checkstack
    // TODO: return to original
    pub unsafe fn pop_value(&self) -> Value {
        match ffi::lua_type(self.state, -1) {
            ffi::LUA_TNIL => {
                ffi::lua_pop(self.state, 1);
                Nil
            }

            ffi::LUA_TBOOLEAN => {
                let b = Value::Boolean(ffi::lua_toboolean(self.state, -1) != 0);
                ffi::lua_pop(self.state, 1);
                b
            }

            ffi::LUA_TLIGHTUSERDATA => {
                let ud = Value::LightUserData(LightUserData(ffi::lua_touserdata(self.state, -1)));
                ffi::lua_pop(self.state, 1);
                ud
            }

            ffi::LUA_TNUMBER => if ffi::lua_isinteger(self.state, -1) != 0 {
                let i = Value::Integer(ffi::lua_tointeger(self.state, -1));
                ffi::lua_pop(self.state, 1);
                i
            } else {
                let n = Value::Number(ffi::lua_tonumber(self.state, -1));
                ffi::lua_pop(self.state, 1);
                n
            },

            ffi::LUA_TSTRING => Value::String(String(self.pop_ref())),

            ffi::LUA_TTABLE => Value::Table(Table(self.pop_ref())),

            ffi::LUA_TFUNCTION => Value::Function(Function(self.pop_ref())),

            ffi::LUA_TUSERDATA => {
                // It should not be possible to interact with userdata types other than custom
                // UserData types OR a WrappedError.  WrappedPanic should never be able to be caught
                // in lua, so it should never be here.
                if let Some(err) = get_wrapped_error(self.state, -1).as_ref() {
                    let err = err.clone();
                    ffi::lua_pop(self.state, 1);
                    Value::Error(err)
                } else {
                    Value::UserData(AnyUserData(self.pop_ref()))
                }
            }

            ffi::LUA_TTHREAD => Value::Thread(Thread(self.pop_ref())),

            _ => rlua_panic!("LUA_TNONE in pop_value"),
        }
    }

    // Pushes a LuaRef value onto the stack, uses 1 stack space, does not call checkstack
    pub(crate) unsafe fn push_ref<'lua>(&'lua self, lref: &LuaRef<'lua>) {
        assert!(
            lref.lua.main_state == self.main_state,
            "Lua instance passed Value created from a different main Lua state"
        );
        let extra = extra_data(self.state);
        ffi::lua_pushvalue((*extra).ref_thread, lref.index);
        ffi::lua_xmove((*extra).ref_thread, self.state, 1);
    }

    // Pops the topmost element of the stack and stores a reference to it.  This pins the object,
    // preventing garbage collection until the returned `LuaRef` is dropped.
    //
    // References are stored in the stack of a specially created auxillary thread that exists only
    // to store reference values.  This is much faster than storing these in the registry, and also
    // much more flexible and requires less bookkeeping than storing them directly in the currently
    // used stack.  The implementation is somewhat biased towards the use case of a relatively small
    // number of short term references being created, and `RegistryKey` being used for long term
    // references.
    pub(crate) unsafe fn pop_ref<'lua>(&'lua self) -> LuaRef<'lua> {
        let extra = extra_data(self.state);
        ffi::lua_xmove(self.state, (*extra).ref_thread, 1);
        let index = ref_stack_pop(extra);
        LuaRef { lua: self, index }
    }

    pub(crate) fn clone_ref<'lua>(&'lua self, lref: &LuaRef<'lua>) -> LuaRef<'lua> {
        unsafe {
            let extra = extra_data(self.state);
            ffi::lua_pushvalue((*extra).ref_thread, lref.index);
            let index = ref_stack_pop(extra);
            LuaRef { lua: self, index }
        }
    }

    pub(crate) fn drop_ref<'lua>(&'lua self, lref: &mut LuaRef<'lua>) {
        unsafe {
            let extra = extra_data(self.state);
            ffi::lua_pushnil((*extra).ref_thread);
            ffi::lua_replace((*extra).ref_thread, lref.index);
            (*extra).ref_free.push(lref.index);
        }
    }

    pub(crate) unsafe fn userdata_metatable<T: 'static + UserData>(&self) -> Result<c_int> {
        if let Some(table_id) = (*extra_data(self.state))
            .registered_userdata
            .get(&TypeId::of::<T>())
        {
            return Ok(*table_id);
        }

        let _sg = StackGuard::new(self.state);
        assert_stack(self.state, 8);

        let mut methods = StaticUserDataMethods::default();
        T::add_methods(&mut methods);

        protect_lua_closure(self.state, 0, 1, |state| {
            ffi::lua_newtable(state);
        })?;
        for (k, m) in methods.meta_methods {
            push_string(self.state, k.name())?;
            self.push_value(Value::Function(self.create_callback(m)?));

            protect_lua_closure(self.state, 3, 1, |state| {
                ffi::lua_rawset(state, -3);
            })?;
        }

        if methods.methods.is_empty() {
            init_userdata_metatable::<RefCell<T>>(self.state, -1, None)?;
        } else {
            protect_lua_closure(self.state, 0, 1, |state| {
                ffi::lua_newtable(state);
            })?;
            for (k, m) in methods.methods {
                push_string(self.state, &k)?;
                self.push_value(Value::Function(self.create_callback(m)?));
                protect_lua_closure(self.state, 3, 1, |state| {
                    ffi::lua_rawset(state, -3);
                })?;
            }

            init_userdata_metatable::<RefCell<T>>(self.state, -2, Some(-1))?;
            ffi::lua_pop(self.state, 1);
        }

        let id = gc_guard(self.state, || {
            ffi::luaL_ref(self.state, ffi::LUA_REGISTRYINDEX)
        });
        (*extra_data(self.state))
            .registered_userdata
            .insert(TypeId::of::<T>(), id);
        Ok(id)
    }

    // Creates a Function out of a Callback containing a 'static Fn.  This is safe ONLY because the
    // Fn is 'static, otherwise it could capture 'callback arguments improperly.  Without ATCs, we
    // cannot easily deal with the "correct" callback type of:
    //
    // Box<for<'lua> Fn(&'lua Lua, MultiValue<'lua>) -> Result<MultiValue<'lua>>)>
    //
    // So we instead use a caller provided lifetime, which without the 'static requirement would be
    // unsafe.
    pub(crate) fn create_callback<'lua, 'callback>(
        &'lua self,
        func: Callback<'callback, 'static>,
    ) -> Result<Function<'lua>> {
        unsafe extern "C" fn call_callback(state: *mut ffi::lua_State) -> c_int {
            callback_error(state, || {
                if ffi::lua_type(state, ffi::lua_upvalueindex(1)) == ffi::LUA_TNIL {
                    return Err(Error::CallbackDestructed);
                }

                let nargs = ffi::lua_gettop(state);
                if nargs < ffi::LUA_MINSTACK {
                    check_stack(state, ffi::LUA_MINSTACK - nargs)?;
                }

                let lua = Lua {
                    state: state,
                    main_state: main_state(state),
                    _phantom: PhantomData,
                };

                let mut args = MultiValue::new();
                args.reserve(nargs as usize);
                for _ in 0..nargs {
                    args.push_front(lua.pop_value());
                }

                let func = get_userdata::<Callback>(state, ffi::lua_upvalueindex(1));

                let results = (*func)(&lua, args)?;
                let nresults = results.len() as c_int;

                check_stack(state, nresults)?;
                for r in results {
                    lua.push_value(r);
                }

                Ok(nresults)
            })
        }

        unsafe {
            let _sg = StackGuard::new(self.state);
            assert_stack(self.state, 4);

            push_userdata::<Callback>(self.state, func)?;

            ffi::lua_pushlightuserdata(
                self.state,
                &FUNCTION_METATABLE_REGISTRY_KEY as *const u8 as *mut c_void,
            );
            ffi::lua_rawget(self.state, ffi::LUA_REGISTRYINDEX);
            ffi::lua_setmetatable(self.state, -2);

            protect_lua_closure(self.state, 1, 1, |state| {
                ffi::lua_pushcclosure(state, call_callback, 1);
            })?;

            Ok(Function(self.pop_ref()))
        }
    }

    // Does not require Send bounds, which can lead to unsafety.
    pub(crate) unsafe fn make_userdata<T>(&self, data: T) -> Result<AnyUserData>
    where
        T: 'static + UserData,
    {
        let _sg = StackGuard::new(self.state);
        assert_stack(self.state, 4);

        let ud_index = self.userdata_metatable::<T>()?;
        push_userdata::<RefCell<T>>(self.state, RefCell::new(data))?;

        ffi::lua_rawgeti(
            self.state,
            ffi::LUA_REGISTRYINDEX,
            ud_index as ffi::lua_Integer,
        );
        ffi::lua_setmetatable(self.state, -2);

        Ok(AnyUserData(self.pop_ref()))
    }
}

// Data associated with the main lua_State via lua_getextraspace.
struct ExtraData {
    registered_userdata: HashMap<TypeId, c_int>,
    registry_unref_list: Arc<Mutex<Option<Vec<c_int>>>>,

    ref_thread: *mut ffi::lua_State,
    ref_stack_size: c_int,
    ref_stack_max: c_int,
    ref_free: Vec<c_int>,
}

unsafe fn extra_data(state: *mut ffi::lua_State) -> *mut ExtraData {
    *(ffi::lua_getextraspace(state) as *mut *mut ExtraData)
}

unsafe fn ref_stack_pop(extra: *mut ExtraData) -> c_int {
    if let Some(free) = (*extra).ref_free.pop() {
        ffi::lua_replace((*extra).ref_thread, free);
        free
    } else {
        if (*extra).ref_stack_max >= (*extra).ref_stack_size {
            // It is a user error to create enough references to exhaust the Lua max stack size for
            // the ref thread.
            if ffi::lua_checkstack((*extra).ref_thread, (*extra).ref_stack_size) == 0 {
                panic!("cannot create a Lua reference, out of auxillary stack space");
            }
            (*extra).ref_stack_size *= 2;
        }
        (*extra).ref_stack_max += 1;
        (*extra).ref_stack_max
    }
}

static FUNCTION_METATABLE_REGISTRY_KEY: u8 = 0;

struct StaticUserDataMethods<'lua, T: 'static + UserData> {
    methods: HashMap<StdString, Callback<'lua, 'static>>,
    meta_methods: HashMap<MetaMethod, Callback<'lua, 'static>>,
    _type: PhantomData<T>,
}

impl<'lua, T: 'static + UserData> Default for StaticUserDataMethods<'lua, T> {
    fn default() -> StaticUserDataMethods<'lua, T> {
        StaticUserDataMethods {
            methods: HashMap::new(),
            meta_methods: HashMap::new(),
            _type: PhantomData,
        }
    }
}

impl<'lua, T: 'static + UserData> UserDataMethods<'lua, T> for StaticUserDataMethods<'lua, T> {
    fn add_method<A, R, M>(&mut self, name: &str, method: M)
    where
        A: FromLuaMulti<'lua>,
        R: ToLuaMulti<'lua>,
        M: 'static + Send + Fn(&'lua Lua, &T, A) -> Result<R>,
    {
        self.methods
            .insert(name.to_owned(), Self::box_method(method));
    }

    fn add_method_mut<A, R, M>(&mut self, name: &str, method: M)
    where
        A: FromLuaMulti<'lua>,
        R: ToLuaMulti<'lua>,
        M: 'static + Send + FnMut(&'lua Lua, &mut T, A) -> Result<R>,
    {
        self.methods
            .insert(name.to_owned(), Self::box_method_mut(method));
    }

    fn add_function<A, R, F>(&mut self, name: &str, function: F)
    where
        A: FromLuaMulti<'lua>,
        R: ToLuaMulti<'lua>,
        F: 'static + Send + Fn(&'lua Lua, A) -> Result<R>,
    {
        self.methods
            .insert(name.to_owned(), Self::box_function(function));
    }

    fn add_function_mut<A, R, F>(&mut self, name: &str, function: F)
    where
        A: FromLuaMulti<'lua>,
        R: ToLuaMulti<'lua>,
        F: 'static + Send + FnMut(&'lua Lua, A) -> Result<R>,
    {
        self.methods
            .insert(name.to_owned(), Self::box_function_mut(function));
    }

    fn add_meta_method<A, R, M>(&mut self, meta: MetaMethod, method: M)
    where
        A: FromLuaMulti<'lua>,
        R: ToLuaMulti<'lua>,
        M: 'static + Send + Fn(&'lua Lua, &T, A) -> Result<R>,
    {
        self.meta_methods.insert(meta, Self::box_method(method));
    }

    fn add_meta_method_mut<A, R, M>(&mut self, meta: MetaMethod, method: M)
    where
        A: FromLuaMulti<'lua>,
        R: ToLuaMulti<'lua>,
        M: 'static + Send + FnMut(&'lua Lua, &mut T, A) -> Result<R>,
    {
        self.meta_methods.insert(meta, Self::box_method_mut(method));
    }

    fn add_meta_function<A, R, F>(&mut self, meta: MetaMethod, function: F)
    where
        A: FromLuaMulti<'lua>,
        R: ToLuaMulti<'lua>,
        F: 'static + Send + Fn(&'lua Lua, A) -> Result<R>,
    {
        self.meta_methods.insert(meta, Self::box_function(function));
    }

    fn add_meta_function_mut<A, R, F>(&mut self, meta: MetaMethod, function: F)
    where
        A: FromLuaMulti<'lua>,
        R: ToLuaMulti<'lua>,
        F: 'static + Send + FnMut(&'lua Lua, A) -> Result<R>,
    {
        self.meta_methods
            .insert(meta, Self::box_function_mut(function));
    }
}

impl<'lua, T: 'static + UserData> StaticUserDataMethods<'lua, T> {
    fn box_method<A, R, M>(method: M) -> Callback<'lua, 'static>
    where
        A: FromLuaMulti<'lua>,
        R: ToLuaMulti<'lua>,
        M: 'static + Send + Fn(&'lua Lua, &T, A) -> Result<R>,
    {
        Box::new(move |lua, mut args| {
            if let Some(front) = args.pop_front() {
                let userdata = AnyUserData::from_lua(front, lua)?;
                let userdata = userdata.borrow::<T>()?;
                method(lua, &userdata, A::from_lua_multi(args, lua)?)?.to_lua_multi(lua)
            } else {
                Err(Error::FromLuaConversionError {
                    from: "missing argument",
                    to: "userdata",
                    message: None,
                })
            }
        })
    }

    fn box_method_mut<A, R, M>(method: M) -> Callback<'lua, 'static>
    where
        A: FromLuaMulti<'lua>,
        R: ToLuaMulti<'lua>,
        M: 'static + Send + FnMut(&'lua Lua, &mut T, A) -> Result<R>,
    {
        let method = RefCell::new(method);
        Box::new(move |lua, mut args| {
            if let Some(front) = args.pop_front() {
                let userdata = AnyUserData::from_lua(front, lua)?;
                let mut userdata = userdata.borrow_mut::<T>()?;
                let mut method = method
                    .try_borrow_mut()
                    .map_err(|_| Error::RecursiveMutCallback)?;
                (&mut *method)(lua, &mut userdata, A::from_lua_multi(args, lua)?)?.to_lua_multi(lua)
            } else {
                Err(Error::FromLuaConversionError {
                    from: "missing argument",
                    to: "userdata",
                    message: None,
                })
            }
        })
    }

    fn box_function<A, R, F>(function: F) -> Callback<'lua, 'static>
    where
        A: FromLuaMulti<'lua>,
        R: ToLuaMulti<'lua>,
        F: 'static + Send + Fn(&'lua Lua, A) -> Result<R>,
    {
        Box::new(move |lua, args| function(lua, A::from_lua_multi(args, lua)?)?.to_lua_multi(lua))
    }

    fn box_function_mut<A, R, F>(function: F) -> Callback<'lua, 'static>
    where
        A: FromLuaMulti<'lua>,
        R: ToLuaMulti<'lua>,
        F: 'static + Send + FnMut(&'lua Lua, A) -> Result<R>,
    {
        let function = RefCell::new(function);
        Box::new(move |lua, args| {
            let function = &mut *function
                .try_borrow_mut()
                .map_err(|_| Error::RecursiveMutCallback)?;
            function(lua, A::from_lua_multi(args, lua)?)?.to_lua_multi(lua)
        })
    }
}
