use std::cell::{Ref, RefMut};

#[cfg(feature = "async")]
use std::future::Future;

#[cfg(feature = "serialize")]
use {
    serde::ser::{self, Serialize, Serializer},
    std::result::Result as StdResult,
};

use crate::error::{Error, Result};
use crate::ffi;
use crate::function::Function;
use crate::lua::Lua;
use crate::table::Table;
use crate::types::{LuaRef, MaybeSend, UserDataCell};
use crate::util::{assert_stack, get_destructed_userdata_metatable, get_userdata, StackGuard};
use crate::value::{FromLua, FromLuaMulti, ToLua, ToLuaMulti, Value};

/// Kinds of metamethods that can be overridden.
///
/// Currently, this mechanism does not allow overriding the `__gc` metamethod, since there is
/// generally no need to do so: [`UserData`] implementors can instead just implement `Drop`.
///
/// [`UserData`]: trait.UserData.html
#[derive(Debug, Copy, Clone, Eq, PartialEq, Hash)]
pub enum MetaMethod {
    /// The `+` operator.
    Add,
    /// The `-` operator.
    Sub,
    /// The `*` operator.
    Mul,
    /// The `/` operator.
    Div,
    /// The `%` operator.
    Mod,
    /// The `^` operator.
    Pow,
    /// The unary minus (`-`) operator.
    Unm,
    /// The floor division (//) operator.
    /// Requires `feature = "lua54/lua53"`
    #[cfg(any(feature = "lua54", feature = "lua53", doc))]
    IDiv,
    /// The bitwise AND (&) operator.
    /// Requires `feature = "lua54/lua53"`
    #[cfg(any(feature = "lua54", feature = "lua53", doc))]
    BAnd,
    /// The bitwise OR (|) operator.
    /// Requires `feature = "lua54/lua53"`
    #[cfg(any(feature = "lua54", feature = "lua53", doc))]
    BOr,
    /// The bitwise XOR (binary ~) operator.
    /// Requires `feature = "lua54/lua53"`
    #[cfg(any(feature = "lua54", feature = "lua53", doc))]
    BXor,
    /// The bitwise NOT (unary ~) operator.
    /// Requires `feature = "lua54/lua53"`
    #[cfg(any(feature = "lua54", feature = "lua53", doc))]
    BNot,
    /// The bitwise left shift (<<) operator.
    #[cfg(any(feature = "lua54", feature = "lua53", doc))]
    Shl,
    /// The bitwise right shift (>>) operator.
    #[cfg(any(feature = "lua54", feature = "lua53", doc))]
    Shr,
    /// The string concatenation operator `..`.
    Concat,
    /// The length operator `#`.
    Len,
    /// The `==` operator.
    Eq,
    /// The `<` operator.
    Lt,
    /// The `<=` operator.
    Le,
    /// Index access `obj[key]`.
    Index,
    /// Index write access `obj[key] = value`.
    NewIndex,
    /// The call "operator" `obj(arg1, args2, ...)`.
    Call,
    /// The `__tostring` metamethod.
    ///
    /// This is not an operator, but will be called by methods such as `tostring` and `print`.
    ToString,
    /// The `__pairs` metamethod.
    ///
    /// This is not an operator, but it will be called by the built-in `pairs` function.
    ///
    /// Requires `feature = "lua54/lua53/lua52"`
    #[cfg(any(feature = "lua54", feature = "lua53", feature = "lua52", doc))]
    Pairs,
    /// The `__close` metamethod.
    ///
    /// Executed when a variable, that marked as to-be-closed, goes out of scope.
    ///
    /// More information about to-be-closed variabled can be found in the Lua 5.4
    /// [documentation][lua_doc].
    ///
    /// Requires `feature = "lua54"`
    ///
    /// [lua_doc]: https://www.lua.org/manual/5.4/manual.html#3.3.8
    #[cfg(any(feature = "lua54", doc))]
    Close,
}

impl MetaMethod {
    pub(crate) fn name(self) -> &'static [u8] {
        match self {
            MetaMethod::Add => b"__add",
            MetaMethod::Sub => b"__sub",
            MetaMethod::Mul => b"__mul",
            MetaMethod::Div => b"__div",
            MetaMethod::Mod => b"__mod",
            MetaMethod::Pow => b"__pow",
            MetaMethod::Unm => b"__unm",

            #[cfg(any(feature = "lua54", feature = "lua53"))]
            MetaMethod::IDiv => b"__idiv",
            #[cfg(any(feature = "lua54", feature = "lua53"))]
            MetaMethod::BAnd => b"__band",
            #[cfg(any(feature = "lua54", feature = "lua53"))]
            MetaMethod::BOr => b"__bor",
            #[cfg(any(feature = "lua54", feature = "lua53"))]
            MetaMethod::BXor => b"__bxor",
            #[cfg(any(feature = "lua54", feature = "lua53"))]
            MetaMethod::BNot => b"__bnot",
            #[cfg(any(feature = "lua54", feature = "lua53"))]
            MetaMethod::Shl => b"__shl",
            #[cfg(any(feature = "lua54", feature = "lua53"))]
            MetaMethod::Shr => b"__shr",

            MetaMethod::Concat => b"__concat",
            MetaMethod::Len => b"__len",
            MetaMethod::Eq => b"__eq",
            MetaMethod::Lt => b"__lt",
            MetaMethod::Le => b"__le",
            MetaMethod::Index => b"__index",
            MetaMethod::NewIndex => b"__newindex",
            MetaMethod::Call => b"__call",
            MetaMethod::ToString => b"__tostring",

            #[cfg(any(feature = "lua54", feature = "lua53", feature = "lua52"))]
            MetaMethod::Pairs => b"__pairs",

            #[cfg(feature = "lua54")]
            MetaMethod::Close => b"__close",
        }
    }
}

/// Method registry for [`UserData`] implementors.
///
/// [`UserData`]: trait.UserData.html
pub trait UserDataMethods<'lua, T: UserData> {
    /// Add a method which accepts a `&T` as the first parameter.
    ///
    /// Regular methods are implemented by overriding the `__index` metamethod and returning the
    /// accessed method. This allows them to be used with the expected `userdata:method()` syntax.
    ///
    /// If `add_meta_method` is used to set the `__index` metamethod, the `__index` metamethod will
    /// be used as a fall-back if no regular method is found.
    fn add_method<S, A, R, M>(&mut self, name: &S, method: M)
    where
        S: ?Sized + AsRef<[u8]>,
        A: FromLuaMulti<'lua>,
        R: ToLuaMulti<'lua>,
        M: 'static + MaybeSend + Fn(&'lua Lua, &T, A) -> Result<R>;

    /// Add a regular method which accepts a `&mut T` as the first parameter.
    ///
    /// Refer to [`add_method`] for more information about the implementation.
    ///
    /// [`add_method`]: #method.add_method
    fn add_method_mut<S, A, R, M>(&mut self, name: &S, method: M)
    where
        S: ?Sized + AsRef<[u8]>,
        A: FromLuaMulti<'lua>,
        R: ToLuaMulti<'lua>,
        M: 'static + MaybeSend + FnMut(&'lua Lua, &mut T, A) -> Result<R>;

    /// Add an async method which accepts a `T` as the first parameter and returns Future.
    /// The passed `T` is cloned from the original value.
    ///
    /// Refer to [`add_method`] for more information about the implementation.
    ///
    /// Requires `feature = "async"`
    ///
    /// [`add_method`]: #method.add_method
    #[cfg(feature = "async")]
    #[cfg_attr(docsrs, doc(cfg(feature = "async")))]
    fn add_async_method<S, A, R, M, MR>(&mut self, name: &S, method: M)
    where
        T: Clone,
        S: ?Sized + AsRef<[u8]>,
        A: FromLuaMulti<'lua>,
        R: ToLuaMulti<'lua>,
        M: 'static + MaybeSend + Fn(&'lua Lua, T, A) -> MR,
        MR: 'lua + Future<Output = Result<R>>;

    /// Add a regular method as a function which accepts generic arguments, the first argument will
    /// be a `UserData` of type T if the method is called with Lua method syntax:
    /// `my_userdata:my_method(arg1, arg2)`, or it is passed in as the first argument:
    /// `my_userdata.my_method(my_userdata, arg1, arg2)`.
    ///
    /// Prefer to use [`add_method`] or [`add_method_mut`] as they are easier to use.
    ///
    /// [`add_method`]: #method.add_method
    /// [`add_method_mut`]: #method.add_method_mut
    fn add_function<S, A, R, F>(&mut self, name: &S, function: F)
    where
        S: ?Sized + AsRef<[u8]>,
        A: FromLuaMulti<'lua>,
        R: ToLuaMulti<'lua>,
        F: 'static + MaybeSend + Fn(&'lua Lua, A) -> Result<R>;

    /// Add a regular method as a mutable function which accepts generic arguments.
    ///
    /// This is a version of [`add_function`] that accepts a FnMut argument.
    ///
    /// [`add_function`]: #method.add_function
    fn add_function_mut<S, A, R, F>(&mut self, name: &S, function: F)
    where
        S: ?Sized + AsRef<[u8]>,
        A: FromLuaMulti<'lua>,
        R: ToLuaMulti<'lua>,
        F: 'static + MaybeSend + FnMut(&'lua Lua, A) -> Result<R>;

    /// Add a regular method as an async function which accepts generic arguments
    /// and returns Future.
    ///
    /// This is an async version of [`add_function`].
    ///
    /// Requires `feature = "async"`
    ///
    /// [`add_function`]: #method.add_function
    #[cfg(feature = "async")]
    #[cfg_attr(docsrs, doc(cfg(feature = "async")))]
    fn add_async_function<S, A, R, F, FR>(&mut self, name: &S, function: F)
    where
        T: Clone,
        S: ?Sized + AsRef<[u8]>,
        A: FromLuaMulti<'lua>,
        R: ToLuaMulti<'lua>,
        F: 'static + MaybeSend + Fn(&'lua Lua, A) -> FR,
        FR: 'lua + Future<Output = Result<R>>;

    /// Add a metamethod which accepts a `&T` as the first parameter.
    ///
    /// # Note
    ///
    /// This can cause an error with certain binary metamethods that can trigger if only the right
    /// side has a metatable. To prevent this, use [`add_meta_function`].
    ///
    /// [`add_meta_function`]: #method.add_meta_function
    fn add_meta_method<A, R, M>(&mut self, meta: MetaMethod, method: M)
    where
        A: FromLuaMulti<'lua>,
        R: ToLuaMulti<'lua>,
        M: 'static + MaybeSend + Fn(&'lua Lua, &T, A) -> Result<R>;

    /// Add a metamethod as a function which accepts a `&mut T` as the first parameter.
    ///
    /// # Note
    ///
    /// This can cause an error with certain binary metamethods that can trigger if only the right
    /// side has a metatable. To prevent this, use [`add_meta_function`].
    ///
    /// [`add_meta_function`]: #method.add_meta_function
    fn add_meta_method_mut<A, R, M>(&mut self, meta: MetaMethod, method: M)
    where
        A: FromLuaMulti<'lua>,
        R: ToLuaMulti<'lua>,
        M: 'static + MaybeSend + FnMut(&'lua Lua, &mut T, A) -> Result<R>;

    /// Add a metamethod which accepts generic arguments.
    ///
    /// Metamethods for binary operators can be triggered if either the left or right argument to
    /// the binary operator has a metatable, so the first argument here is not necessarily a
    /// userdata of type `T`.
    fn add_meta_function<A, R, F>(&mut self, meta: MetaMethod, function: F)
    where
        A: FromLuaMulti<'lua>,
        R: ToLuaMulti<'lua>,
        F: 'static + MaybeSend + Fn(&'lua Lua, A) -> Result<R>;

    /// Add a metamethod as a mutable function which accepts generic arguments.
    ///
    /// This is a version of [`add_meta_function`] that accepts a FnMut argument.
    ///
    /// [`add_meta_function`]: #method.add_meta_function
    fn add_meta_function_mut<A, R, F>(&mut self, meta: MetaMethod, function: F)
    where
        A: FromLuaMulti<'lua>,
        R: ToLuaMulti<'lua>,
        F: 'static + MaybeSend + FnMut(&'lua Lua, A) -> Result<R>;
}

/// Trait for custom userdata types.
///
/// By implementing this trait, a struct becomes eligible for use inside Lua code. Implementations
/// of [`ToLua`] and [`FromLua`] are automatically provided.
///
/// # Examples
///
/// ```
/// # use mlua::{Lua, Result, UserData};
/// # fn main() -> Result<()> {
/// # let lua = Lua::new();
/// struct MyUserData(i32);
///
/// impl UserData for MyUserData {}
///
/// // `MyUserData` now implements `ToLua`:
/// lua.globals().set("myobject", MyUserData(123))?;
///
/// lua.load("assert(type(myobject) == 'userdata')").exec()?;
/// # Ok(())
/// # }
/// ```
///
/// Custom methods and operators can be provided by implementing `add_methods` (refer to
/// [`UserDataMethods`] for more information):
///
/// ```
/// # use mlua::{Lua, MetaMethod, Result, UserData, UserDataMethods};
/// # fn main() -> Result<()> {
/// # let lua = Lua::new();
/// struct MyUserData(i32);
///
/// impl UserData for MyUserData {
///     fn add_methods<'lua, M: UserDataMethods<'lua, Self>>(methods: &mut M) {
///         methods.add_method("get", |_, this, _: ()| {
///             Ok(this.0)
///         });
///
///         methods.add_method_mut("add", |_, this, value: i32| {
///             this.0 += value;
///             Ok(())
///         });
///
///         methods.add_meta_method(MetaMethod::Add, |_, this, value: i32| {
///             Ok(this.0 + value)
///         });
///     }
/// }
///
/// lua.globals().set("myobject", MyUserData(123))?;
///
/// lua.load(r#"
///     assert(myobject:get() == 123)
///     myobject:add(7)
///     assert(myobject:get() == 130)
///     assert(myobject + 10 == 140)
/// "#).exec()?;
/// # Ok(())
/// # }
/// ```
///
/// [`ToLua`]: trait.ToLua.html
/// [`FromLua`]: trait.FromLua.html
/// [`UserDataMethods`]: trait.UserDataMethods.html
pub trait UserData: Sized {
    /// Adds custom methods and operators specific to this userdata.
    fn add_methods<'lua, M: UserDataMethods<'lua, Self>>(_methods: &mut M) {}
}

pub(crate) struct UserDataWrapped<T> {
    pub(crate) data: *mut T,
    #[cfg(feature = "serialize")]
    ser: *mut dyn erased_serde::Serialize,
}

impl<T> Drop for UserDataWrapped<T> {
    fn drop(&mut self) {
        unsafe {
            drop(Box::from_raw(self.data));
            #[cfg(feature = "serialize")]
            if self.data as *mut () != self.ser as *mut () {
                drop(Box::from_raw(self.ser));
            }
        }
    }
}

impl<T> UserDataWrapped<T> {
    pub(crate) fn new(data: T) -> Self {
        UserDataWrapped {
            data: Box::into_raw(Box::new(data)),
            #[cfg(feature = "serialize")]
            ser: Box::into_raw(Box::new(UserDataSerializeError)),
        }
    }

    #[cfg(feature = "serialize")]
    pub(crate) fn new_ser(data: T) -> Self
    where
        T: 'static + Serialize,
    {
        let data_raw = Box::into_raw(Box::new(data));
        UserDataWrapped {
            data: data_raw,
            ser: data_raw,
        }
    }
}

impl<T> AsRef<T> for UserDataWrapped<T> {
    fn as_ref(&self) -> &T {
        unsafe { &*self.data }
    }
}

impl<T> AsMut<T> for UserDataWrapped<T> {
    fn as_mut(&mut self) -> &mut T {
        unsafe { &mut *self.data }
    }
}

#[cfg(feature = "serialize")]
pub(crate) struct UserDataSerializeError;

#[cfg(feature = "serialize")]
impl Serialize for UserDataSerializeError {
    fn serialize<S>(&self, _serializer: S) -> StdResult<S::Ok, S::Error>
    where
        S: Serializer,
    {
        Err(ser::Error::custom("cannot serialize <userdata>"))
    }
}

/// Handle to an internal Lua userdata for any type that implements [`UserData`].
///
/// Similar to `std::any::Any`, this provides an interface for dynamic type checking via the [`is`]
/// and [`borrow`] methods.
///
/// Internally, instances are stored in a `RefCell`, to best match the mutable semantics of the Lua
/// language.
///
/// # Note
///
/// This API should only be used when necessary. Implementing [`UserData`] already allows defining
/// methods which check the type and acquire a borrow behind the scenes.
///
/// [`UserData`]: trait.UserData.html
/// [`is`]: #method.is
/// [`borrow`]: #method.borrow
#[derive(Clone, Debug)]
pub struct AnyUserData<'lua>(pub(crate) LuaRef<'lua>);

impl<'lua> AnyUserData<'lua> {
    /// Checks whether the type of this userdata is `T`.
    pub fn is<T: 'static + UserData>(&self) -> bool {
        match self.inspect(|_: &UserDataCell<T>| Ok(())) {
            Ok(()) => true,
            Err(Error::UserDataTypeMismatch) => false,
            Err(_) => unreachable!(),
        }
    }

    /// Borrow this userdata immutably if it is of type `T`.
    ///
    /// # Errors
    ///
    /// Returns a `UserDataBorrowError` if the userdata is already mutably borrowed. Returns a
    /// `UserDataTypeMismatch` if the userdata is not of type `T`.
    pub fn borrow<T: 'static + UserData>(&self) -> Result<Ref<T>> {
        self.inspect(|cell| {
            let cell_ref = cell.try_borrow().map_err(|_| Error::UserDataBorrowError)?;
            Ok(Ref::map(cell_ref, |x| unsafe { &*x.data }))
        })
    }

    /// Borrow this userdata mutably if it is of type `T`.
    ///
    /// # Errors
    ///
    /// Returns a `UserDataBorrowMutError` if the userdata is already borrowed. Returns a
    /// `UserDataTypeMismatch` if the userdata is not of type `T`.
    pub fn borrow_mut<T: 'static + UserData>(&self) -> Result<RefMut<T>> {
        self.inspect(|cell| {
            let cell_ref = cell
                .try_borrow_mut()
                .map_err(|_| Error::UserDataBorrowMutError)?;
            Ok(RefMut::map(cell_ref, |x| unsafe { &mut *x.data }))
        })
    }

    /// Sets an associated value to this `AnyUserData`.
    ///
    /// The value may be any Lua value whatsoever, and can be retrieved with [`get_user_value`].
    /// As Lua < 5.3 allows to store only tables, the value will be stored in a table at index 1.
    ///
    /// [`get_user_value`]: #method.get_user_value
    pub fn set_user_value<V: ToLua<'lua>>(&self, v: V) -> Result<()> {
        let lua = self.0.lua;
        #[cfg(any(feature = "lua52", feature = "lua51", feature = "luajit"))]
        let v = {
            // Lua 5.2/5.1 allows to store only a table. Then we will wrap the value.
            let t = lua.create_table()?;
            t.raw_set(1, v)?;
            crate::Value::Table(t)
        };
        #[cfg(any(feature = "lua54", feature = "lua53"))]
        let v = v.to_lua(lua)?;
        unsafe {
            let _sg = StackGuard::new(lua.state);
            assert_stack(lua.state, 2);
            lua.push_ref(&self.0);
            lua.push_value(v)?;
            ffi::lua_setuservalue(lua.state, -2);
            Ok(())
        }
    }

    /// Returns an associated value set by [`set_user_value`].
    ///
    /// For Lua < 5.3 the value will be automatically extracted from the table wrapper from index 1.
    ///
    /// [`set_user_value`]: #method.set_user_value
    pub fn get_user_value<V: FromLua<'lua>>(&self) -> Result<V> {
        let lua = self.0.lua;
        let res = unsafe {
            let _sg = StackGuard::new(lua.state);
            assert_stack(lua.state, 3);
            lua.push_ref(&self.0);
            ffi::lua_getuservalue(lua.state, -1);
            lua.pop_value()
        };
        #[cfg(any(feature = "lua52", feature = "lua51", feature = "luajit"))]
        return crate::Table::from_lua(res, lua)?.get(1);
        #[cfg(any(feature = "lua54", feature = "lua53"))]
        V::from_lua(res, lua)
    }

    /// Checks for a metamethod in this `AnyUserData`
    pub fn has_metamethod(&self, method: MetaMethod) -> Result<bool> {
        match self.get_metatable() {
            Ok(mt) => {
                let name = self.0.lua.create_string(method.name())?;
                if let Value::Nil = mt.raw_get(name)? {
                    Ok(false)
                } else {
                    Ok(true)
                }
            }
            Err(Error::UserDataTypeMismatch) => Ok(false),
            Err(e) => Err(e),
        }
    }

    fn get_metatable(&self) -> Result<Table<'lua>> {
        unsafe {
            let lua = self.0.lua;
            let _sg = StackGuard::new(lua.state);
            assert_stack(lua.state, 3);

            lua.push_ref(&self.0);

            if ffi::lua_getmetatable(lua.state, -1) == 0 {
                return Err(Error::UserDataTypeMismatch);
            }

            Ok(Table(lua.pop_ref()))
        }
    }

    pub(crate) fn equals<T: AsRef<Self>>(&self, other: T) -> Result<bool> {
        let other = other.as_ref();
        if self == other {
            return Ok(true);
        }

        let mt = self.get_metatable()?;
        if mt != other.get_metatable()? {
            return Ok(false);
        }

        if mt.contains_key("__eq")? {
            return mt
                .get::<_, Function>("__eq")?
                .call((self.clone(), other.clone()));
        }

        Ok(false)
    }

    fn inspect<'a, T, R, F>(&'a self, func: F) -> Result<R>
    where
        T: 'static + UserData,
        F: FnOnce(&'a UserDataCell<T>) -> Result<R>,
    {
        unsafe {
            let lua = self.0.lua;
            let _sg = StackGuard::new(lua.state);
            assert_stack(lua.state, 3);

            lua.push_ref(&self.0);

            if ffi::lua_getmetatable(lua.state, -1) == 0 {
                Err(Error::UserDataTypeMismatch)
            } else {
                ffi::lua_rawgeti(
                    lua.state,
                    ffi::LUA_REGISTRYINDEX,
                    lua.userdata_metatable::<T>()? as ffi::lua_Integer,
                );

                if ffi::lua_rawequal(lua.state, -1, -2) == 0 {
                    // Maybe UserData destructed?
                    ffi::lua_pop(lua.state, 1);
                    get_destructed_userdata_metatable(lua.state);
                    if ffi::lua_rawequal(lua.state, -1, -2) == 1 {
                        Err(Error::UserDataDestructed)
                    } else {
                        Err(Error::UserDataTypeMismatch)
                    }
                } else {
                    func(&*get_userdata::<UserDataCell<T>>(lua.state, -3))
                }
            }
        }
    }
}

impl<'lua> PartialEq for AnyUserData<'lua> {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

impl<'lua> AsRef<AnyUserData<'lua>> for AnyUserData<'lua> {
    #[inline]
    fn as_ref(&self) -> &Self {
        self
    }
}

#[cfg(feature = "serialize")]
impl<'lua> Serialize for AnyUserData<'lua> {
    fn serialize<S>(&self, serializer: S) -> StdResult<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let f = || unsafe {
            let lua = self.0.lua;
            let _sg = StackGuard::new(lua.state);
            assert_stack(lua.state, 2);

            lua.push_userdata_ref(&self.0)?;
            let ud = &*get_userdata::<UserDataCell<()>>(lua.state, -1);
            (*ud.try_borrow().map_err(|_| Error::UserDataBorrowError)?.ser)
                .serialize(serializer)
                .map_err(|err| Error::SerializeError(err.to_string()))
        };
        f().map_err(ser::Error::custom)
    }
}
