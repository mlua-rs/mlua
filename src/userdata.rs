use std::cell::{Ref, RefMut};
use std::fmt;
use std::hash::{Hash, Hasher};
use std::string::String as StdString;

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
use crate::table::{Table, TablePairs};
use crate::types::{LuaRef, MaybeSend, UserDataCell};
use crate::util::{assert_stack, get_destructed_userdata_metatable, get_userdata, StackGuard};
use crate::value::{FromLua, FromLuaMulti, ToLua, ToLuaMulti, Value};

/// Kinds of metamethods that can be overridden.
///
/// Currently, this mechanism does not allow overriding the `__gc` metamethod, since there is
/// generally no need to do so: [`UserData`] implementors can instead just implement `Drop`.
///
/// [`UserData`]: trait.UserData.html
#[derive(Debug, Clone)]
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
    /// A custom metamethod.
    ///
    /// Must not be in the protected list: `__gc`, `__metatable`.
    Custom(StdString),
}

impl PartialEq for MetaMethod {
    fn eq(&self, other: &Self) -> bool {
        self.name() == other.name()
    }
}

impl Eq for MetaMethod {}

impl Hash for MetaMethod {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.name().hash(state);
    }
}

impl fmt::Display for MetaMethod {
    fn fmt(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(fmt, "{}", self.name())
    }
}

impl MetaMethod {
    pub(crate) fn name(&self) -> &str {
        match self {
            MetaMethod::Add => "__add",
            MetaMethod::Sub => "__sub",
            MetaMethod::Mul => "__mul",
            MetaMethod::Div => "__div",
            MetaMethod::Mod => "__mod",
            MetaMethod::Pow => "__pow",
            MetaMethod::Unm => "__unm",

            #[cfg(any(feature = "lua54", feature = "lua53"))]
            MetaMethod::IDiv => "__idiv",
            #[cfg(any(feature = "lua54", feature = "lua53"))]
            MetaMethod::BAnd => "__band",
            #[cfg(any(feature = "lua54", feature = "lua53"))]
            MetaMethod::BOr => "__bor",
            #[cfg(any(feature = "lua54", feature = "lua53"))]
            MetaMethod::BXor => "__bxor",
            #[cfg(any(feature = "lua54", feature = "lua53"))]
            MetaMethod::BNot => "__bnot",
            #[cfg(any(feature = "lua54", feature = "lua53"))]
            MetaMethod::Shl => "__shl",
            #[cfg(any(feature = "lua54", feature = "lua53"))]
            MetaMethod::Shr => "__shr",

            MetaMethod::Concat => "__concat",
            MetaMethod::Len => "__len",
            MetaMethod::Eq => "__eq",
            MetaMethod::Lt => "__lt",
            MetaMethod::Le => "__le",
            MetaMethod::Index => "__index",
            MetaMethod::NewIndex => "__newindex",
            MetaMethod::Call => "__call",
            MetaMethod::ToString => "__tostring",

            #[cfg(any(feature = "lua54", feature = "lua53", feature = "lua52"))]
            MetaMethod::Pairs => "__pairs",

            #[cfg(feature = "lua54")]
            MetaMethod::Close => "__close",

            MetaMethod::Custom(ref name) => name,
        }
    }

    pub(crate) fn validate(self) -> Result<Self> {
        match self {
            MetaMethod::Custom(name) if name == "__gc" => Err(Error::MetaMethodRestricted(name)),
            MetaMethod::Custom(name) if name == "__metatable" => {
                Err(Error::MetaMethodRestricted(name))
            }
            _ => Ok(self),
        }
    }
}

impl From<StdString> for MetaMethod {
    fn from(name: StdString) -> Self {
        match name.as_str() {
            "__add" => MetaMethod::Add,
            "__sub" => MetaMethod::Sub,
            "__mul" => MetaMethod::Mul,
            "__div" => MetaMethod::Div,
            "__mod" => MetaMethod::Mod,
            "__pow" => MetaMethod::Pow,
            "__unm" => MetaMethod::Unm,

            #[cfg(any(feature = "lua54", feature = "lua53"))]
            "__idiv" => MetaMethod::IDiv,
            #[cfg(any(feature = "lua54", feature = "lua53"))]
            "__band" => MetaMethod::BAnd,
            #[cfg(any(feature = "lua54", feature = "lua53"))]
            "__bor" => MetaMethod::BOr,
            #[cfg(any(feature = "lua54", feature = "lua53"))]
            "__bxor" => MetaMethod::BXor,
            #[cfg(any(feature = "lua54", feature = "lua53"))]
            "__bnot" => MetaMethod::BNot,
            #[cfg(any(feature = "lua54", feature = "lua53"))]
            "__shl" => MetaMethod::Shl,
            #[cfg(any(feature = "lua54", feature = "lua53"))]
            "__shr" => MetaMethod::Shr,

            "__concat" => MetaMethod::Concat,
            "__len" => MetaMethod::Len,
            "__eq" => MetaMethod::Eq,
            "__lt" => MetaMethod::Lt,
            "__le" => MetaMethod::Le,
            "__index" => MetaMethod::Index,
            "__newindex" => MetaMethod::NewIndex,
            "__call" => MetaMethod::Call,
            "__tostring" => MetaMethod::ToString,

            #[cfg(any(feature = "lua54", feature = "lua53", feature = "lua52"))]
            "__pairs" => MetaMethod::Pairs,

            #[cfg(feature = "lua54")]
            "__close" => MetaMethod::Close,

            _ => MetaMethod::Custom(name),
        }
    }
}

impl From<&str> for MetaMethod {
    fn from(name: &str) -> Self {
        MetaMethod::from(name.to_owned())
    }
}

/// Method registry for [`UserData`] implementors.
///
/// [`UserData`]: trait.UserData.html
pub trait UserDataMethods<'lua, T: UserData> {
    /// Add a regular method which accepts a `&T` as the first parameter.
    ///
    /// Regular methods are implemented by overriding the `__index` metamethod and returning the
    /// accessed method. This allows them to be used with the expected `userdata:method()` syntax.
    ///
    /// If `add_meta_method` is used to set the `__index` metamethod, the `__index` metamethod will
    /// be used as a fall-back if no regular method is found.
    fn add_method<S, A, R, M>(&mut self, name: &S, method: M)
    where
        S: AsRef<[u8]> + ?Sized,
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
        S: AsRef<[u8]> + ?Sized,
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
        S: AsRef<[u8]> + ?Sized,
        A: FromLuaMulti<'lua>,
        R: ToLuaMulti<'lua>,
        M: 'static + MaybeSend + Fn(&'lua Lua, T, A) -> MR,
        MR: 'lua + Future<Output = Result<R>>;

    /// Add a regular method as a function which accepts generic arguments, the first argument will
    /// be a [`AnyUserData`] of type `T` if the method is called with Lua method syntax:
    /// `my_userdata:my_method(arg1, arg2)`, or it is passed in as the first argument:
    /// `my_userdata.my_method(my_userdata, arg1, arg2)`.
    ///
    /// Prefer to use [`add_method`] or [`add_method_mut`] as they are easier to use.
    ///
    /// [`AnyUserData`]: struct.AnyUserData.html
    /// [`add_method`]: #method.add_method
    /// [`add_method_mut`]: #method.add_method_mut
    fn add_function<S, A, R, F>(&mut self, name: &S, function: F)
    where
        S: AsRef<[u8]> + ?Sized,
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
        S: AsRef<[u8]> + ?Sized,
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
        S: AsRef<[u8]> + ?Sized,
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
    fn add_meta_method<S, A, R, M>(&mut self, meta: S, method: M)
    where
        S: Into<MetaMethod>,
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
    fn add_meta_method_mut<S, A, R, M>(&mut self, meta: S, method: M)
    where
        S: Into<MetaMethod>,
        A: FromLuaMulti<'lua>,
        R: ToLuaMulti<'lua>,
        M: 'static + MaybeSend + FnMut(&'lua Lua, &mut T, A) -> Result<R>;

    /// Add a metamethod which accepts generic arguments.
    ///
    /// Metamethods for binary operators can be triggered if either the left or right argument to
    /// the binary operator has a metatable, so the first argument here is not necessarily a
    /// userdata of type `T`.
    fn add_meta_function<S, A, R, F>(&mut self, meta: S, function: F)
    where
        S: Into<MetaMethod>,
        A: FromLuaMulti<'lua>,
        R: ToLuaMulti<'lua>,
        F: 'static + MaybeSend + Fn(&'lua Lua, A) -> Result<R>;

    /// Add a metamethod as a mutable function which accepts generic arguments.
    ///
    /// This is a version of [`add_meta_function`] that accepts a FnMut argument.
    ///
    /// [`add_meta_function`]: #method.add_meta_function
    fn add_meta_function_mut<S, A, R, F>(&mut self, meta: S, function: F)
    where
        S: Into<MetaMethod>,
        A: FromLuaMulti<'lua>,
        R: ToLuaMulti<'lua>,
        F: 'static + MaybeSend + FnMut(&'lua Lua, A) -> Result<R>;
}

/// Field registry for [`UserData`] implementors.
///
/// [`UserData`]: trait.UserData.html
pub trait UserDataFields<'lua, T: UserData> {
    /// Add a regular field getter as a method which accepts a `&T` as the parameter.
    ///
    /// Regular field getters are implemented by overriding the `__index` metamethod and returning the
    /// accessed field. This allows them to be used with the expected `userdata.field` syntax.
    ///
    /// If `add_meta_method` is used to set the `__index` metamethod, the `__index` metamethod will
    /// be used as a fall-back if no regular field or method are found.
    fn add_field_method_get<S, R, M>(&mut self, name: &S, method: M)
    where
        S: AsRef<[u8]> + ?Sized,
        R: ToLua<'lua>,
        M: 'static + MaybeSend + Fn(&'lua Lua, &T) -> Result<R>;

    /// Add a regular field setter as a method which accepts a `&mut T` as the first parameter.
    ///
    /// Regular field setters are implemented by overriding the `__newindex` metamethod and setting the
    /// accessed field. This allows them to be used with the expected `userdata.field = value` syntax.
    ///
    /// If `add_meta_method` is used to set the `__newindex` metamethod, the `__newindex` metamethod will
    /// be used as a fall-back if no regular field is found.
    fn add_field_method_set<S, A, M>(&mut self, name: &S, method: M)
    where
        S: AsRef<[u8]> + ?Sized,
        A: FromLua<'lua>,
        M: 'static + MaybeSend + FnMut(&'lua Lua, &mut T, A) -> Result<()>;

    /// Add a regular field getter as a function which accepts a generic [`AnyUserData`] of type `T`
    /// argument.
    ///
    /// Prefer to use [`add_field_method_get`] as it is easier to use.
    ///
    /// [`AnyUserData`]: struct.AnyUserData.html
    /// [`add_field_method_get`]: #method.add_field_method_get
    fn add_field_function_get<S, R, F>(&mut self, name: &S, function: F)
    where
        S: AsRef<[u8]> + ?Sized,
        R: ToLua<'lua>,
        F: 'static + MaybeSend + Fn(&'lua Lua, AnyUserData<'lua>) -> Result<R>;

    /// Add a regular field setter as a function which accepts a generic [`AnyUserData`] of type `T`
    /// first argument.
    ///
    /// Prefer to use [`add_field_method_set`] as it is easier to use.
    ///
    /// [`AnyUserData`]: struct.AnyUserData.html
    /// [`add_field_method_set`]: #method.add_field_method_set
    fn add_field_function_set<S, A, F>(&mut self, name: &S, function: F)
    where
        S: AsRef<[u8]> + ?Sized,
        A: FromLua<'lua>,
        F: 'static + MaybeSend + FnMut(&'lua Lua, AnyUserData<'lua>, A) -> Result<()>;

    /// Add a metamethod value computed from `f`.
    ///
    /// This will initialize the metamethod value from `f` on `UserData` creation.
    ///
    /// # Note
    ///
    /// `mlua` will trigger an error on an attempt to define a protected metamethod,
    /// like `__gc` or `__metatable`.
    fn add_meta_field_with<S, R, F>(&mut self, meta: S, f: F)
    where
        S: Into<MetaMethod>,
        F: 'static + MaybeSend + Fn(&'lua Lua) -> Result<R>,
        R: ToLua<'lua>;
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
/// Custom fields, methods and operators can be provided by implementing `add_fields` or `add_methods`
/// (refer to [`UserDataFields`] and [`UserDataMethods`] for more information):
///
/// ```
/// # use mlua::{Lua, MetaMethod, Result, UserData, UserDataFields, UserDataMethods};
/// # fn main() -> Result<()> {
/// # let lua = Lua::new();
/// struct MyUserData(i32);
///
/// impl UserData for MyUserData {
///     fn add_fields<'lua, F: UserDataFields<'lua, Self>>(fields: &mut F) {
///         fields.add_field_method_get("val", |_, this| Ok(this.0));
///     }
///
///     fn add_methods<'lua, M: UserDataMethods<'lua, Self>>(methods: &mut M) {
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
///     assert(myobject.val == 123)
///     myobject:add(7)
///     assert(myobject.val == 130)
///     assert(myobject + 10 == 140)
/// "#).exec()?;
/// # Ok(())
/// # }
/// ```
///
/// [`ToLua`]: trait.ToLua.html
/// [`FromLua`]: trait.FromLua.html
/// [`UserDataFields`]: trait.UserDataFields.html
/// [`UserDataMethods`]: trait.UserDataMethods.html
pub trait UserData: Sized {
    /// Adds custom fields specific to this userdata.
    fn add_fields<'lua, F: UserDataFields<'lua, Self>>(_fields: &mut F) {}

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

    /// Returns a metatable of this `UserData`.
    ///
    /// Returned [`UserDataMetatable`] object wraps the original metatable and
    /// allows to provide safe access to it methods.
    ///
    /// [`UserDataMetatable`]: struct.UserDataMetatable.html
    pub fn get_metatable(&self) -> Result<UserDataMetatable<'lua>> {
        self.get_raw_metatable().map(UserDataMetatable)
    }

    /// Checks for a metamethod in this `AnyUserData`.
    ///
    /// This function is deprecated and will be removed in v0.7.
    /// Please use [`get_metatable`] function instead.
    ///
    /// [`get_metatable`]: #method.get_metatable
    #[deprecated(
        since = "0.6.0",
        note = "Please use the get_metatable function instead"
    )]
    pub fn has_metamethod(&self, method: MetaMethod) -> Result<bool> {
        match self.get_raw_metatable() {
            Ok(mt) => {
                let name = self.0.lua.create_string(method.validate()?.name())?;
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

    fn get_raw_metatable(&self) -> Result<Table<'lua>> {
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
        // Uses lua_rawequal() under the hood
        if self == other {
            return Ok(true);
        }

        let mt = self.get_raw_metatable()?;
        if mt != other.get_raw_metatable()? {
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

/// Handle to a `UserData` metatable.
#[derive(Clone, Debug)]
pub struct UserDataMetatable<'lua>(pub(crate) Table<'lua>);

impl<'lua> UserDataMetatable<'lua> {
    /// Gets the value associated to `key` from the metatable.
    ///
    /// If no value is associated to `key`, returns the `Nil` value.
    /// Access to restricted metamethods such as `__gc` or `__metatable` will cause an error.
    pub fn get<K: Into<MetaMethod>, V: FromLua<'lua>>(&self, key: K) -> Result<V> {
        self.0.raw_get(key.into().validate()?.name())
    }

    /// Sets a key-value pair in the metatable.
    ///
    /// If the value is `Nil`, this will effectively remove the `key`.
    /// Access to restricted metamethods such as `__gc` or `__metatable` will cause an error.
    /// Setting `__index` or `__newindex` metamethods is also restricted because their values are cached
    /// for `mlua` internal usage.
    pub fn set<K: Into<MetaMethod>, V: ToLua<'lua>>(&self, key: K, value: V) -> Result<()> {
        let key = key.into().validate()?;
        // `__index` and `__newindex` cannot be changed in runtime, because values are cached
        if key == MetaMethod::Index || key == MetaMethod::NewIndex {
            return Err(Error::MetaMethodRestricted(key.to_string()));
        }
        self.0.raw_set(key.name(), value)
    }

    /// Checks whether the metatable contains a non-nil value for `key`.
    pub fn contains<K: Into<MetaMethod>>(&self, key: K) -> Result<bool> {
        self.0.contains_key(key.into().validate()?.name())
    }

    /// Consumes this metatable and returns an iterator over the pairs of the metatable.
    ///
    /// The pairs are wrapped in a [`Result`], since they are lazily converted to `V` type.
    ///
    /// [`Result`]: type.Result.html
    pub fn pairs<K: FromLua<'lua>, V: FromLua<'lua>>(self) -> UserDataMetatablePairs<'lua, V> {
        UserDataMetatablePairs(self.0.pairs())
    }
}

/// An iterator over the pairs of a [`UserData`] metatable.
///
/// It skips restricted metamethods, such as `__gc` or `__metatable`.
///
/// This struct is created by the [`UserDataMetatable::pairs`] method.
///
/// [`UserData`]: trait.UserData.html
/// [`UserDataMetatable::pairs`]: struct.UserDataMetatable.html#method.pairs
pub struct UserDataMetatablePairs<'lua, V>(TablePairs<'lua, StdString, V>);

impl<'lua, V> Iterator for UserDataMetatablePairs<'lua, V>
where
    V: FromLua<'lua>,
{
    type Item = Result<(MetaMethod, V)>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            match self.0.next()? {
                Ok((key, value)) => {
                    // Skip restricted metamethods
                    if let Ok(metamethod) = MetaMethod::from(key).validate() {
                        break Some(Ok((metamethod, value)));
                    }
                }
                Err(e) => break Some(Err(e)),
            }
        }
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
