use std::any::TypeId;
use std::cell::{Ref, RefCell, RefMut};
use std::fmt;
use std::hash::{Hash, Hasher};
use std::ops::{Deref, DerefMut};
use std::os::raw::{c_char, c_int};
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
use crate::types::{Callback, LuaRef, MaybeSend};
use crate::util::{check_stack, get_userdata, take_userdata, StackGuard};
use crate::value::{FromLua, FromLuaMulti, ToLua, ToLuaMulti};

#[cfg(feature = "async")]
use crate::types::AsyncCallback;

#[cfg(feature = "lua54")]
pub(crate) const USER_VALUE_MAXSLOT: usize = 8;

/// Kinds of metamethods that can be overridden.
///
/// Currently, this mechanism does not allow overriding the `__gc` metamethod, since there is
/// generally no need to do so: [`UserData`] implementors can instead just implement `Drop`.
///
/// [`UserData`]: crate::UserData
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
    #[cfg(any(feature = "lua54", feature = "lua53"))]
    IDiv,
    /// The bitwise AND (&) operator.
    /// Requires `feature = "lua54/lua53"`
    #[cfg(any(feature = "lua54", feature = "lua53"))]
    BAnd,
    /// The bitwise OR (|) operator.
    /// Requires `feature = "lua54/lua53"`
    #[cfg(any(feature = "lua54", feature = "lua53"))]
    BOr,
    /// The bitwise XOR (binary ~) operator.
    /// Requires `feature = "lua54/lua53"`
    #[cfg(any(feature = "lua54", feature = "lua53"))]
    BXor,
    /// The bitwise NOT (unary ~) operator.
    /// Requires `feature = "lua54/lua53"`
    #[cfg(any(feature = "lua54", feature = "lua53"))]
    BNot,
    /// The bitwise left shift (<<) operator.
    #[cfg(any(feature = "lua54", feature = "lua53"))]
    Shl,
    /// The bitwise right shift (>>) operator.
    #[cfg(any(feature = "lua54", feature = "lua53"))]
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
    #[cfg(any(
        feature = "lua54",
        feature = "lua53",
        feature = "lua52",
        feature = "luajit52",
    ))]
    Pairs,
    /// The `__ipairs` metamethod.
    ///
    /// This is not an operator, but it will be called by the built-in [`ipairs`] function.
    ///
    /// Requires `feature = "lua52"`
    ///
    /// [`ipairs`]: https://www.lua.org/manual/5.2/manual.html#pdf-ipairs
    #[cfg(any(feature = "lua52", feature = "luajit52", doc))]
    #[cfg_attr(docsrs, doc(cfg(any(feature = "lua52", feature = "luajit52"))))]
    IPairs,
    /// The `__iter` metamethod.
    ///
    /// Executed before the iteration begins, and should return an iterator function like `next`
    /// (or a custom one).
    ///
    /// Requires `feature = "lua"`
    #[cfg(any(feature = "luau", doc))]
    #[cfg_attr(docsrs, doc(cfg(feature = "luau")))]
    Iter,
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
    #[cfg(any(feature = "lua54"))]
    Close,
    /// A custom metamethod.
    ///
    /// Must not be in the protected list: `__gc`, `__metatable`, `__mlua*`.
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
    /// Returns Lua metamethod name, usually prefixed by two underscores.
    pub fn name(&self) -> &str {
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

            #[cfg(any(
                feature = "lua54",
                feature = "lua53",
                feature = "lua52",
                feature = "luajit52"
            ))]
            MetaMethod::Pairs => "__pairs",
            #[cfg(any(feature = "lua52", feature = "luajit52"))]
            MetaMethod::IPairs => "__ipairs",
            #[cfg(feature = "luau")]
            MetaMethod::Iter => "__iter",

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
            MetaMethod::Custom(name) if name.starts_with("__mlua") => {
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

            #[cfg(any(
                feature = "lua54",
                feature = "lua53",
                feature = "lua52",
                feature = "luajit52"
            ))]
            "__pairs" => MetaMethod::Pairs,
            #[cfg(any(feature = "lua52", feature = "luajit52"))]
            "__ipairs" => MetaMethod::IPairs,
            #[cfg(feature = "luau")]
            "__iter" => MetaMethod::Iter,

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
/// [`UserData`]: crate::UserData
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
    /// [`AnyUserData`]: crate::AnyUserData
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

    /// Add an async metamethod which accepts a `T` as the first parameter and returns Future.
    /// The passed `T` is cloned from the original value.
    ///
    /// This is an async version of [`add_meta_method`].
    ///
    /// Requires `feature = "async"`
    ///
    /// [`add_meta_method`]: #method.add_meta_method
    #[cfg(all(feature = "async", not(any(feature = "lua51", feature = "luau"))))]
    #[cfg_attr(docsrs, doc(cfg(feature = "async")))]
    fn add_async_meta_method<S, A, R, M, MR>(&mut self, name: S, method: M)
    where
        T: Clone,
        S: Into<MetaMethod>,
        A: FromLuaMulti<'lua>,
        R: ToLuaMulti<'lua>,
        M: 'static + MaybeSend + Fn(&'lua Lua, T, A) -> MR,
        MR: 'lua + Future<Output = Result<R>>;

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

    /// Add a metamethod which accepts generic arguments and returns Future.
    ///
    /// This is an async version of [`add_meta_function`].
    ///
    /// Requires `feature = "async"`
    ///
    /// [`add_meta_function`]: #method.add_meta_function
    #[cfg(all(feature = "async", not(any(feature = "lua51", feature = "luau"))))]
    #[cfg_attr(docsrs, doc(cfg(feature = "async")))]
    fn add_async_meta_function<S, A, R, F, FR>(&mut self, name: S, function: F)
    where
        S: Into<MetaMethod>,
        A: FromLuaMulti<'lua>,
        R: ToLuaMulti<'lua>,
        F: 'static + MaybeSend + Fn(&'lua Lua, A) -> FR,
        FR: 'lua + Future<Output = Result<R>>;

    //
    // Below are internal methods used in generated code
    //

    #[doc(hidden)]
    fn add_callback(&mut self, _name: Vec<u8>, _callback: Callback<'lua, 'static>) {}

    #[doc(hidden)]
    #[cfg(feature = "async")]
    fn add_async_callback(&mut self, _name: Vec<u8>, _callback: AsyncCallback<'lua, 'static>) {}

    #[doc(hidden)]
    fn add_meta_callback(&mut self, _meta: MetaMethod, _callback: Callback<'lua, 'static>) {}

    #[doc(hidden)]
    #[cfg(feature = "async")]
    fn add_async_meta_callback(
        &mut self,
        _meta: MetaMethod,
        _callback: AsyncCallback<'lua, 'static>,
    ) {
    }
}

/// Field registry for [`UserData`] implementors.
///
/// [`UserData`]: crate::UserData
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
    /// [`AnyUserData`]: crate::AnyUserData
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
    /// [`AnyUserData`]: crate::AnyUserData
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

    //
    // Below are internal methods used in generated code
    //

    #[doc(hidden)]
    fn add_field_getter(&mut self, _name: Vec<u8>, _callback: Callback<'lua, 'static>) {}

    #[doc(hidden)]
    fn add_field_setter(&mut self, _name: Vec<u8>, _callback: Callback<'lua, 'static>) {}
}

/// Trait for custom userdata types.
///
/// By implementing this trait, a struct becomes eligible for use inside Lua code.
/// Implementation of [`ToLua`] is automatically provided, [`FromLua`] is implemented
/// only for `T: UserData + Clone`.
///
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
/// [`ToLua`]: crate::ToLua
/// [`FromLua`]: crate::FromLua
/// [`UserDataFields`]: crate::UserDataFields
/// [`UserDataMethods`]: crate::UserDataMethods
pub trait UserData: Sized {
    /// Adds custom fields specific to this userdata.
    fn add_fields<'lua, F: UserDataFields<'lua, Self>>(_fields: &mut F) {}

    /// Adds custom methods and operators specific to this userdata.
    fn add_methods<'lua, M: UserDataMethods<'lua, Self>>(_methods: &mut M) {}
}

// Wraps UserData in a way to always implement `serde::Serialize` trait.
pub(crate) struct UserDataCell<T>(RefCell<UserDataWrapped<T>>);

impl<T> UserDataCell<T> {
    #[inline]
    pub(crate) fn new(data: T) -> Self {
        UserDataCell(RefCell::new(UserDataWrapped::new(data)))
    }

    #[cfg(feature = "serialize")]
    #[inline]
    pub(crate) fn new_ser(data: T) -> Self
    where
        T: 'static + Serialize,
    {
        UserDataCell(RefCell::new(UserDataWrapped::new_ser(data)))
    }

    // Immutably borrows the wrapped value.
    #[inline]
    pub(crate) fn try_borrow(&self) -> Result<Ref<T>> {
        self.0
            .try_borrow()
            .map(|r| Ref::map(r, |r| r.deref()))
            .map_err(|_| Error::UserDataBorrowError)
    }

    // Mutably borrows the wrapped value.
    #[inline]
    pub(crate) fn try_borrow_mut(&self) -> Result<RefMut<T>> {
        self.0
            .try_borrow_mut()
            .map(|r| RefMut::map(r, |r| r.deref_mut()))
            .map_err(|_| Error::UserDataBorrowMutError)
    }

    // Consumes this `UserDataCell`, returning the wrapped value.
    #[inline]
    fn into_inner(self) -> T {
        self.0.into_inner().into_inner()
    }
}

pub(crate) enum UserDataWrapped<T> {
    Default(Box<T>),
    #[cfg(feature = "serialize")]
    Serializable(Box<dyn erased_serde::Serialize>),
}

impl<T> UserDataWrapped<T> {
    #[inline]
    fn new(data: T) -> Self {
        UserDataWrapped::Default(Box::new(data))
    }

    #[cfg(feature = "serialize")]
    #[inline]
    fn new_ser(data: T) -> Self
    where
        T: 'static + Serialize,
    {
        UserDataWrapped::Serializable(Box::new(data))
    }

    #[inline]
    fn into_inner(self) -> T {
        match self {
            Self::Default(data) => *data,
            #[cfg(feature = "serialize")]
            Self::Serializable(data) => unsafe { *Box::from_raw(Box::into_raw(data) as *mut T) },
        }
    }
}

impl<T> Deref for UserDataWrapped<T> {
    type Target = T;

    #[inline]
    fn deref(&self) -> &Self::Target {
        match self {
            Self::Default(data) => data,
            #[cfg(feature = "serialize")]
            Self::Serializable(data) => unsafe {
                &*(data.as_ref() as *const _ as *const Self::Target)
            },
        }
    }
}

impl<T> DerefMut for UserDataWrapped<T> {
    #[inline]
    fn deref_mut(&mut self) -> &mut Self::Target {
        match self {
            Self::Default(data) => data,
            #[cfg(feature = "serialize")]
            Self::Serializable(data) => unsafe {
                &mut *(data.as_mut() as *mut _ as *mut Self::Target)
            },
        }
    }
}

#[cfg(feature = "serialize")]
struct UserDataSerializeError;

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
/// [`UserData`]: crate::UserData
/// [`is`]: crate::AnyUserData::is
/// [`borrow`]: crate::AnyUserData::borrow
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
    #[inline]
    pub fn borrow<T: 'static + UserData>(&self) -> Result<Ref<T>> {
        self.inspect(|cell| cell.try_borrow())
    }

    /// Borrow this userdata mutably if it is of type `T`.
    ///
    /// # Errors
    ///
    /// Returns a `UserDataBorrowMutError` if the userdata cannot be mutably borrowed.
    /// Returns a `UserDataTypeMismatch` if the userdata is not of type `T`.
    #[inline]
    pub fn borrow_mut<T: 'static + UserData>(&self) -> Result<RefMut<T>> {
        self.inspect(|cell| cell.try_borrow_mut())
    }

    /// Takes out the value of `UserData` and sets the special "destructed" metatable that prevents
    /// any further operations with this userdata.
    ///
    /// All associated user values will be also cleared.
    pub fn take<T: 'static + UserData>(&self) -> Result<T> {
        let lua = self.0.lua;
        unsafe {
            let _sg = StackGuard::new(lua.state);
            check_stack(lua.state, 3)?;

            let type_id = lua.push_userdata_ref(&self.0)?;
            match type_id {
                Some(type_id) if type_id == TypeId::of::<T>() => {
                    // Try to borrow userdata exclusively
                    let _ = (*get_userdata::<UserDataCell<T>>(lua.state, -1)).try_borrow_mut()?;

                    // Clear associated user values
                    #[cfg(feature = "lua54")]
                    for i in 1..=USER_VALUE_MAXSLOT {
                        ffi::lua_pushnil(lua.state);
                        ffi::lua_setiuservalue(lua.state, -2, i as c_int);
                    }
                    #[cfg(any(feature = "lua53", feature = "lua52", feature = "luau"))]
                    {
                        ffi::lua_pushnil(lua.state);
                        ffi::lua_setuservalue(lua.state, -2);
                    }
                    #[cfg(any(feature = "lua51", feature = "luajit"))]
                    protect_lua!(lua.state, 1, 1, fn(state) {
                        ffi::lua_newtable(state);
                        ffi::lua_setuservalue(state, -2);
                    })?;

                    Ok(take_userdata::<UserDataCell<T>>(lua.state).into_inner())
                }
                _ => Err(Error::UserDataTypeMismatch),
            }
        }
    }

    /// Sets an associated value to this `AnyUserData`.
    ///
    /// The value may be any Lua value whatsoever, and can be retrieved with [`get_user_value`].
    ///
    /// This is the same as calling [`set_nth_user_value`] with `n` set to 1.
    ///
    /// [`get_user_value`]: #method.get_user_value
    /// [`set_nth_user_value`]: #method.set_nth_user_value
    #[inline]
    pub fn set_user_value<V: ToLua<'lua>>(&self, v: V) -> Result<()> {
        self.set_nth_user_value(1, v)
    }

    /// Returns an associated value set by [`set_user_value`].
    ///
    /// This is the same as calling [`get_nth_user_value`] with `n` set to 1.
    ///
    /// [`set_user_value`]: #method.set_user_value
    /// [`get_nth_user_value`]: #method.get_nth_user_value
    #[inline]
    pub fn get_user_value<V: FromLua<'lua>>(&self) -> Result<V> {
        self.get_nth_user_value(1)
    }

    /// Sets an associated `n`th value to this `AnyUserData`.
    ///
    /// The value may be any Lua value whatsoever, and can be retrieved with [`get_nth_user_value`].
    /// `n` starts from 1 and can be up to 65535.
    ///
    /// This is supported for all Lua versions.
    /// In Lua 5.4 first 7 elements are stored in a most efficient way.
    /// For other Lua versions this functionality is provided using a wrapping table.
    ///
    /// [`get_nth_user_value`]: #method.get_nth_user_value
    pub fn set_nth_user_value<V: ToLua<'lua>>(&self, n: usize, v: V) -> Result<()> {
        if n < 1 || n > u16::MAX as usize {
            return Err(Error::RuntimeError(
                "user value index out of bounds".to_string(),
            ));
        }

        let lua = self.0.lua;
        unsafe {
            let _sg = StackGuard::new(lua.state);
            check_stack(lua.state, 5)?;

            lua.push_userdata_ref(&self.0)?;
            lua.push_value(v.to_lua(lua)?)?;

            #[cfg(feature = "lua54")]
            if n < USER_VALUE_MAXSLOT {
                ffi::lua_setiuservalue(lua.state, -2, n as c_int);
                return Ok(());
            }

            // Multiple (extra) user values are emulated by storing them in a table
            protect_lua!(lua.state, 2, 0, |state| {
                if getuservalue_table(state, -2) != ffi::LUA_TTABLE {
                    // Create a new table to use as uservalue
                    ffi::lua_pop(state, 1);
                    ffi::lua_newtable(state);
                    ffi::lua_pushvalue(state, -1);

                    #[cfg(feature = "lua54")]
                    ffi::lua_setiuservalue(state, -4, USER_VALUE_MAXSLOT as c_int);
                    #[cfg(not(feature = "lua54"))]
                    ffi::lua_setuservalue(state, -4);
                }
                ffi::lua_pushvalue(state, -2);
                #[cfg(feature = "lua54")]
                ffi::lua_rawseti(state, -2, (n - USER_VALUE_MAXSLOT + 1) as ffi::lua_Integer);
                #[cfg(not(feature = "lua54"))]
                ffi::lua_rawseti(state, -2, n as ffi::lua_Integer);
            })?;

            Ok(())
        }
    }

    /// Returns an associated `n`th value set by [`set_nth_user_value`].
    ///
    /// `n` starts from 1 and can be up to 65535.
    ///
    /// This is supported for all Lua versions.
    /// In Lua 5.4 first 7 elements are stored in a most efficient way.
    /// For other Lua versions this functionality is provided using a wrapping table.
    ///
    /// [`set_nth_user_value`]: #method.set_nth_user_value
    pub fn get_nth_user_value<V: FromLua<'lua>>(&self, n: usize) -> Result<V> {
        if n < 1 || n > u16::MAX as usize {
            return Err(Error::RuntimeError(
                "user value index out of bounds".to_string(),
            ));
        }

        let lua = self.0.lua;
        unsafe {
            let _sg = StackGuard::new(lua.state);
            check_stack(lua.state, 4)?;

            lua.push_userdata_ref(&self.0)?;

            #[cfg(feature = "lua54")]
            if n < USER_VALUE_MAXSLOT {
                ffi::lua_getiuservalue(lua.state, -1, n as c_int);
                return V::from_lua(lua.pop_value(), lua);
            }

            // Multiple (extra) user values are emulated by storing them in a table
            protect_lua!(lua.state, 1, 1, |state| {
                if getuservalue_table(state, -1) != ffi::LUA_TTABLE {
                    ffi::lua_pushnil(state);
                    return;
                }
                #[cfg(feature = "lua54")]
                ffi::lua_rawgeti(state, -1, (n - USER_VALUE_MAXSLOT + 1) as ffi::lua_Integer);
                #[cfg(not(feature = "lua54"))]
                ffi::lua_rawgeti(state, -1, n as ffi::lua_Integer);
            })?;

            V::from_lua(lua.pop_value(), lua)
        }
    }

    /// Sets an associated value to this `AnyUserData` by name.
    ///
    /// The value can be retrieved with [`get_named_user_value`].
    ///
    /// [`get_named_user_value`]: #method.get_named_user_value
    pub fn set_named_user_value<S, V>(&self, name: &S, v: V) -> Result<()>
    where
        S: AsRef<[u8]> + ?Sized,
        V: ToLua<'lua>,
    {
        let lua = self.0.lua;
        unsafe {
            let _sg = StackGuard::new(lua.state);
            check_stack(lua.state, 5)?;

            lua.push_userdata_ref(&self.0)?;
            lua.push_value(v.to_lua(lua)?)?;

            // Multiple (extra) user values are emulated by storing them in a table
            let name = name.as_ref();
            protect_lua!(lua.state, 2, 0, |state| {
                if getuservalue_table(state, -2) != ffi::LUA_TTABLE {
                    // Create a new table to use as uservalue
                    ffi::lua_pop(state, 1);
                    ffi::lua_newtable(state);
                    ffi::lua_pushvalue(state, -1);

                    #[cfg(feature = "lua54")]
                    ffi::lua_setiuservalue(state, -4, USER_VALUE_MAXSLOT as c_int);
                    #[cfg(not(feature = "lua54"))]
                    ffi::lua_setuservalue(state, -4);
                }
                ffi::lua_pushlstring(state, name.as_ptr() as *const c_char, name.len());
                ffi::lua_pushvalue(state, -3);
                ffi::lua_rawset(state, -3);
            })?;

            Ok(())
        }
    }

    /// Returns an associated value by name set by [`set_named_user_value`].
    ///
    /// [`set_named_user_value`]: #method.set_named_user_value
    pub fn get_named_user_value<S, V>(&self, name: &S) -> Result<V>
    where
        S: AsRef<[u8]> + ?Sized,
        V: FromLua<'lua>,
    {
        let lua = self.0.lua;
        unsafe {
            let _sg = StackGuard::new(lua.state);
            check_stack(lua.state, 4)?;

            lua.push_userdata_ref(&self.0)?;

            // Multiple (extra) user values are emulated by storing them in a table
            let name = name.as_ref();
            protect_lua!(lua.state, 1, 1, |state| {
                if getuservalue_table(state, -1) != ffi::LUA_TTABLE {
                    ffi::lua_pushnil(state);
                    return;
                }
                ffi::lua_pushlstring(state, name.as_ptr() as *const c_char, name.len());
                ffi::lua_rawget(state, -2);
            })?;

            V::from_lua(lua.pop_value(), lua)
        }
    }

    /// Returns a metatable of this `UserData`.
    ///
    /// Returned [`UserDataMetatable`] object wraps the original metatable and
    /// provides safe access to its methods.
    ///
    /// For `T: UserData + 'static` returned metatable is shared among all instances of type `T`.
    ///
    /// [`UserDataMetatable`]: crate::UserDataMetatable
    pub fn get_metatable(&self) -> Result<UserDataMetatable<'lua>> {
        self.get_raw_metatable().map(UserDataMetatable)
    }

    fn get_raw_metatable(&self) -> Result<Table<'lua>> {
        unsafe {
            let lua = self.0.lua;
            let _sg = StackGuard::new(lua.state);
            check_stack(lua.state, 3)?;

            lua.push_userdata_ref(&self.0)?;
            ffi::lua_getmetatable(lua.state, -1); // Checked that non-empty on the previous call
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
        let lua = self.0.lua;
        unsafe {
            let _sg = StackGuard::new(lua.state);
            check_stack(lua.state, 2)?;

            let type_id = lua.push_userdata_ref(&self.0)?;
            match type_id {
                Some(type_id) if type_id == TypeId::of::<T>() => {
                    func(&*get_userdata::<UserDataCell<T>>(lua.state, -1))
                }
                _ => Err(Error::UserDataTypeMismatch),
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

unsafe fn getuservalue_table(state: *mut ffi::lua_State, idx: c_int) -> c_int {
    #[cfg(feature = "lua54")]
    return ffi::lua_getiuservalue(state, idx, USER_VALUE_MAXSLOT as c_int);
    #[cfg(not(feature = "lua54"))]
    return ffi::lua_getuservalue(state, idx);
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
    /// [`Result`]: crate::Result
    pub fn pairs<V: FromLua<'lua>>(self) -> UserDataMetatablePairs<'lua, V> {
        UserDataMetatablePairs(self.0.pairs())
    }
}

/// An iterator over the pairs of a [`UserData`] metatable.
///
/// It skips restricted metamethods, such as `__gc` or `__metatable`.
///
/// This struct is created by the [`UserDataMetatable::pairs`] method.
///
/// [`UserData`]: crate::UserData
/// [`UserDataMetatable::pairs`]: crate::UserDataMetatable::method.pairs
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
        let lua = self.0.lua;
        let data = unsafe {
            let _sg = StackGuard::new(lua.state);
            check_stack(lua.state, 3).map_err(ser::Error::custom)?;

            lua.push_userdata_ref(&self.0).map_err(ser::Error::custom)?;
            let ud = &*get_userdata::<UserDataCell<()>>(lua.state, -1);
            ud.0.try_borrow()
                .map_err(|_| ser::Error::custom(Error::UserDataBorrowError))?
        };
        match &*data {
            UserDataWrapped::Default(_) => UserDataSerializeError.serialize(serializer),
            UserDataWrapped::Serializable(ser) => ser.serialize(serializer),
        }
    }
}
