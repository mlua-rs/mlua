use std::cell::RefCell;
use std::os::raw::{c_int, c_void};
use std::{mem, ptr, slice};

use crate::error::{Error, Result};
use crate::state::Lua;
use crate::table::Table;
use crate::traits::{FromLuaMulti, IntoLua, IntoLuaMulti, LuaNativeFn, LuaNativeFnMut};
use crate::types::{Callback, LuaType, MaybeSend, ValueRef};
use crate::util::{
    assert_stack, check_stack, linenumber_to_usize, pop_error, ptr_to_lossy_str, ptr_to_str, StackGuard,
};
use crate::value::Value;

#[cfg(feature = "async")]
use {
    crate::thread::AsyncThread,
    crate::traits::LuaNativeAsyncFn,
    crate::types::AsyncCallback,
    std::future::{self, Future},
    std::pin::Pin,
    std::task::{Context, Poll},
};

/// Handle to an internal Lua function.
#[derive(Clone, Debug, PartialEq)]
pub struct Function(pub(crate) ValueRef);

/// Contains information about a function.
///
/// Please refer to the [`Lua Debug Interface`] for more information.
///
/// [`Lua Debug Interface`]: https://www.lua.org/manual/5.4/manual.html#4.7
#[derive(Clone, Debug)]
pub struct FunctionInfo {
    /// A (reasonable) name of the function (`None` if the name cannot be found).
    pub name: Option<String>,
    /// Explains the `name` field (can be `global`/`local`/`method`/`field`/`upvalue`/etc).
    ///
    /// Always `None` for Luau.
    pub name_what: Option<&'static str>,
    /// A string `Lua` if the function is a Lua function, `C` if it is a C function, `main` if it is
    /// the main part of a chunk.
    pub what: &'static str,
    /// Source of the chunk that created the function.
    pub source: Option<String>,
    /// A "printable" version of `source`, to be used in error messages.
    pub short_src: Option<String>,
    /// The line number where the definition of the function starts.
    pub line_defined: Option<usize>,
    /// The line number where the definition of the function ends (not set by Luau).
    pub last_line_defined: Option<usize>,
}

/// Luau function coverage snapshot.
#[cfg(any(feature = "luau", doc))]
#[cfg_attr(docsrs, doc(cfg(feature = "luau")))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CoverageInfo {
    pub function: Option<String>,
    pub line_defined: i32,
    pub depth: i32,
    pub hits: Vec<i32>,
}

impl Function {
    /// Calls the function, passing `args` as function arguments.
    ///
    /// The function's return values are converted to the generic type `R`.
    ///
    /// # Examples
    ///
    /// Call Lua's built-in `tostring` function:
    ///
    /// ```
    /// # use mlua::{Function, Lua, Result};
    /// # fn main() -> Result<()> {
    /// # let lua = Lua::new();
    /// let globals = lua.globals();
    ///
    /// let tostring: Function = globals.get("tostring")?;
    ///
    /// assert_eq!(tostring.call::<String>(123)?, "123");
    ///
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// Call a function with multiple arguments:
    ///
    /// ```
    /// # use mlua::{Function, Lua, Result};
    /// # fn main() -> Result<()> {
    /// # let lua = Lua::new();
    /// let sum: Function = lua.load(
    ///     r#"
    ///         function(a, b)
    ///             return a + b
    ///         end
    /// "#).eval()?;
    ///
    /// assert_eq!(sum.call::<u32>((3, 4))?, 3 + 4);
    ///
    /// # Ok(())
    /// # }
    /// ```
    pub fn call<R: FromLuaMulti>(&self, args: impl IntoLuaMulti) -> Result<R> {
        let lua = self.0.lua.lock();
        let state = lua.state();
        unsafe {
            let _sg = StackGuard::new(state);
            check_stack(state, 2)?;

            // Push error handler
            lua.push_error_traceback();
            let stack_start = ffi::lua_gettop(state);
            // Push function and the arguments
            lua.push_ref(&self.0);
            let nargs = args.push_into_stack_multi(&lua)?;
            // Call the function
            let ret = ffi::lua_pcall(state, nargs, ffi::LUA_MULTRET, stack_start);
            if ret != ffi::LUA_OK {
                return Err(pop_error(state, ret));
            }
            // Get the results
            let nresults = ffi::lua_gettop(state) - stack_start;
            R::from_stack_multi(nresults, &lua)
        }
    }

    /// Returns a future that, when polled, calls `self`, passing `args` as function arguments,
    /// and drives the execution.
    ///
    /// Internally it wraps the function to an [`AsyncThread`]. The returned type implements
    /// `Future<Output = Result<R>>` and can be awaited.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::time::Duration;
    /// # use mlua::{Lua, Result};
    /// # #[tokio::main]
    /// # async fn main() -> Result<()> {
    /// # let lua = Lua::new();
    ///
    /// let sleep = lua.create_async_function(move |_lua, n: u64| async move {
    ///     tokio::time::sleep(Duration::from_millis(n)).await;
    ///     Ok(())
    /// })?;
    ///
    /// sleep.call_async::<()>(10).await?;
    ///
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// [`AsyncThread`]: crate::AsyncThread
    #[cfg(feature = "async")]
    #[cfg_attr(docsrs, doc(cfg(feature = "async")))]
    pub fn call_async<R>(&self, args: impl IntoLuaMulti) -> AsyncCallFuture<R>
    where
        R: FromLuaMulti,
    {
        let lua = self.0.lua.lock();
        AsyncCallFuture(unsafe {
            lua.create_recycled_thread(self).and_then(|th| {
                let mut th = th.into_async(args)?;
                th.set_recyclable(true);
                Ok(th)
            })
        })
    }

    /// Returns a function that, when called, calls `self`, passing `args` as the first set of
    /// arguments.
    ///
    /// If any arguments are passed to the returned function, they will be passed after `args`.
    ///
    /// # Examples
    ///
    /// ```
    /// # use mlua::{Function, Lua, Result};
    /// # fn main() -> Result<()> {
    /// # let lua = Lua::new();
    /// let sum: Function = lua.load(
    ///     r#"
    ///         function(a, b)
    ///             return a + b
    ///         end
    /// "#).eval()?;
    ///
    /// let bound_a = sum.bind(1)?;
    /// assert_eq!(bound_a.call::<u32>(2)?, 1 + 2);
    ///
    /// let bound_a_and_b = sum.bind(13)?.bind(57)?;
    /// assert_eq!(bound_a_and_b.call::<u32>(())?, 13 + 57);
    ///
    /// # Ok(())
    /// # }
    /// ```
    pub fn bind(&self, args: impl IntoLuaMulti) -> Result<Function> {
        unsafe extern "C-unwind" fn args_wrapper_impl(state: *mut ffi::lua_State) -> c_int {
            let nargs = ffi::lua_gettop(state);
            let nbinds = ffi::lua_tointeger(state, ffi::lua_upvalueindex(1)) as c_int;
            ffi::luaL_checkstack(state, nbinds, ptr::null());

            for i in 0..nbinds {
                ffi::lua_pushvalue(state, ffi::lua_upvalueindex(i + 2));
            }
            if nargs > 0 {
                ffi::lua_rotate(state, 1, nbinds);
            }

            nargs + nbinds
        }

        let lua = self.0.lua.lock();
        let state = lua.state();

        let args = args.into_lua_multi(lua.lua())?;
        let nargs = args.len() as c_int;

        if nargs == 0 {
            return Ok(self.clone());
        }

        if nargs + 1 > ffi::LUA_MAX_UPVALUES {
            return Err(Error::BindError);
        }

        let args_wrapper = unsafe {
            let _sg = StackGuard::new(state);
            check_stack(state, nargs + 3)?;

            ffi::lua_pushinteger(state, nargs as ffi::lua_Integer);
            for arg in &args {
                lua.push_value(arg)?;
            }
            protect_lua!(state, nargs + 1, 1, fn(state) {
                ffi::lua_pushcclosure(state, args_wrapper_impl, ffi::lua_gettop(state));
            })?;

            Function(lua.pop_ref())
        };

        let lua = lua.lua();
        lua.load(
            r#"
            local func, args_wrapper = ...
            return function(...)
                return func(args_wrapper(...))
            end
            "#,
        )
        .try_cache()
        .set_name("=__mlua_bind")
        .call((self, args_wrapper))
    }

    /// Returns the environment of the Lua function.
    ///
    /// By default Lua functions shares a global environment.
    ///
    /// This function always returns `None` for Rust/C functions.
    pub fn environment(&self) -> Option<Table> {
        let lua = self.0.lua.lock();
        let state = lua.state();
        unsafe {
            let _sg = StackGuard::new(state);
            assert_stack(state, 1);

            lua.push_ref(&self.0);
            if ffi::lua_iscfunction(state, -1) != 0 {
                return None;
            }

            #[cfg(any(feature = "lua51", feature = "luajit", feature = "luau"))]
            ffi::lua_getfenv(state, -1);
            #[cfg(any(feature = "lua54", feature = "lua53", feature = "lua52"))]
            for i in 1..=255 {
                // Traverse upvalues until we find the _ENV one
                match ffi::lua_getupvalue(state, -1, i) {
                    s if s.is_null() => break,
                    s if std::ffi::CStr::from_ptr(s as _) == c"_ENV" => break,
                    _ => ffi::lua_pop(state, 1),
                }
            }

            if ffi::lua_type(state, -1) != ffi::LUA_TTABLE {
                return None;
            }
            Some(Table(lua.pop_ref()))
        }
    }

    /// Sets the environment of the Lua function.
    ///
    /// The environment is a table that is used as the global environment for the function.
    /// Returns `true` if environment successfully changed, `false` otherwise.
    ///
    /// This function does nothing for Rust/C functions.
    pub fn set_environment(&self, env: Table) -> Result<bool> {
        let lua = self.0.lua.lock();
        let state = lua.state();
        unsafe {
            let _sg = StackGuard::new(state);
            check_stack(state, 2)?;

            lua.push_ref(&self.0);
            if ffi::lua_iscfunction(state, -1) != 0 {
                return Ok(false);
            }

            #[cfg(any(feature = "lua51", feature = "luajit", feature = "luau"))]
            {
                lua.push_ref(&env.0);
                ffi::lua_setfenv(state, -2);
            }
            #[cfg(any(feature = "lua54", feature = "lua53", feature = "lua52"))]
            for i in 1..=255 {
                match ffi::lua_getupvalue(state, -1, i) {
                    s if s.is_null() => return Ok(false),
                    s if std::ffi::CStr::from_ptr(s as _) == c"_ENV" => {
                        ffi::lua_pop(state, 1);
                        // Create an anonymous function with the new environment
                        let f_with_env = lua
                            .lua()
                            .load("return _ENV")
                            .set_environment(env)
                            .try_cache()
                            .into_function()?;
                        lua.push_ref(&f_with_env.0);
                        ffi::lua_upvaluejoin(state, -2, i, -1, 1);
                        break;
                    }
                    _ => ffi::lua_pop(state, 1),
                }
            }

            Ok(true)
        }
    }

    /// Returns information about the function.
    ///
    /// Corresponds to the `>Sn` what mask for [`lua_getinfo`] when applied to the function.
    ///
    /// [`lua_getinfo`]: https://www.lua.org/manual/5.4/manual.html#lua_getinfo
    pub fn info(&self) -> FunctionInfo {
        let lua = self.0.lua.lock();
        let state = lua.state();
        unsafe {
            let _sg = StackGuard::new(state);
            assert_stack(state, 1);

            let mut ar: ffi::lua_Debug = mem::zeroed();
            lua.push_ref(&self.0);
            #[cfg(not(feature = "luau"))]
            let res = ffi::lua_getinfo(state, cstr!(">Sn"), &mut ar);
            #[cfg(feature = "luau")]
            let res = ffi::lua_getinfo(state, -1, cstr!("sn"), &mut ar);
            mlua_assert!(res != 0, "lua_getinfo failed with `>Sn`");

            FunctionInfo {
                name: ptr_to_lossy_str(ar.name).map(|s| s.into_owned()),
                #[cfg(not(feature = "luau"))]
                name_what: match ptr_to_str(ar.namewhat) {
                    Some("") => None,
                    val => val,
                },
                #[cfg(feature = "luau")]
                name_what: None,
                what: ptr_to_str(ar.what).unwrap_or("main"),
                source: ptr_to_lossy_str(ar.source).map(|s| s.into_owned()),
                #[cfg(not(feature = "luau"))]
                short_src: ptr_to_lossy_str(ar.short_src.as_ptr()).map(|s| s.into_owned()),
                #[cfg(feature = "luau")]
                short_src: ptr_to_lossy_str(ar.short_src).map(|s| s.into_owned()),
                line_defined: linenumber_to_usize(ar.linedefined),
                #[cfg(not(feature = "luau"))]
                last_line_defined: linenumber_to_usize(ar.lastlinedefined),
                #[cfg(feature = "luau")]
                last_line_defined: None,
            }
        }
    }

    /// Dumps the function as a binary chunk.
    ///
    /// If `strip` is true, the binary representation may not include all debug information
    /// about the function, to save space.
    ///
    /// For Luau a [`Compiler`] can be used to compile Lua chunks to bytecode.
    ///
    /// [`Compiler`]: crate::chunk::Compiler
    #[cfg(not(feature = "luau"))]
    #[cfg_attr(docsrs, doc(cfg(not(feature = "luau"))))]
    pub fn dump(&self, strip: bool) -> Vec<u8> {
        unsafe extern "C-unwind" fn writer(
            _state: *mut ffi::lua_State,
            buf: *const c_void,
            buf_len: usize,
            data: *mut c_void,
        ) -> c_int {
            let data = &mut *(data as *mut Vec<u8>);
            let buf = slice::from_raw_parts(buf as *const u8, buf_len);
            data.extend_from_slice(buf);
            0
        }

        let lua = self.0.lua.lock();
        let state = lua.state();
        let mut data: Vec<u8> = Vec::new();
        unsafe {
            let _sg = StackGuard::new(state);
            assert_stack(state, 1);

            lua.push_ref(&self.0);
            let data_ptr = &mut data as *mut Vec<u8> as *mut c_void;
            ffi::lua_dump(state, writer, data_ptr, strip as i32);
            ffi::lua_pop(state, 1);
        }

        data
    }

    /// Retrieves recorded coverage information about this Lua function including inner calls.
    ///
    /// This function takes a callback as an argument and calls it providing [`CoverageInfo`]
    /// snapshot per each executed inner function.
    ///
    /// Recording of coverage information is controlled by [`Compiler::set_coverage_level`] option.
    ///
    /// [`Compiler::set_coverage_level`]: crate::chunk::Compiler::set_coverage_level
    #[cfg(any(feature = "luau", doc))]
    #[cfg_attr(docsrs, doc(cfg(feature = "luau")))]
    pub fn coverage<F>(&self, func: F)
    where
        F: FnMut(CoverageInfo),
    {
        use std::ffi::CStr;
        use std::os::raw::c_char;

        unsafe extern "C-unwind" fn callback<F: FnMut(CoverageInfo)>(
            data: *mut c_void,
            function: *const c_char,
            line_defined: c_int,
            depth: c_int,
            hits: *const c_int,
            size: usize,
        ) {
            let function = if !function.is_null() {
                Some(CStr::from_ptr(function).to_string_lossy().to_string())
            } else {
                None
            };
            let rust_callback = &*(data as *const RefCell<F>);
            if let Ok(mut rust_callback) = rust_callback.try_borrow_mut() {
                // Call the Rust callback with CoverageInfo
                rust_callback(CoverageInfo {
                    function,
                    line_defined,
                    depth,
                    hits: slice::from_raw_parts(hits, size).to_vec(),
                });
            }
        }

        let lua = self.0.lua.lock();
        let state = lua.state();
        unsafe {
            let _sg = StackGuard::new(state);
            assert_stack(state, 1);

            lua.push_ref(&self.0);
            let func = RefCell::new(func);
            let func_ptr = &func as *const RefCell<F> as *mut c_void;
            ffi::lua_getcoverage(state, -1, func_ptr, callback::<F>);
        }
    }

    /// Converts this function to a generic C pointer.
    ///
    /// There is no way to convert the pointer back to its original value.
    ///
    /// Typically this function is used only for hashing and debug information.
    #[inline]
    pub fn to_pointer(&self) -> *const c_void {
        self.0.to_pointer()
    }

    /// Creates a deep clone of the Lua function.
    ///
    /// Copies the function prototype and all its upvalues to the
    /// newly created function.
    /// This function returns shallow clone (same handle) for Rust/C functions.
    #[cfg(any(feature = "luau", doc))]
    #[cfg_attr(docsrs, doc(cfg(feature = "luau")))]
    pub fn deep_clone(&self) -> Result<Self> {
        let lua = self.0.lua.lock();
        let state = lua.state();
        unsafe {
            let _sg = StackGuard::new(state);
            check_stack(state, 2)?;

            lua.push_ref(&self.0);
            if ffi::lua_iscfunction(state, -1) != 0 {
                return Ok(self.clone());
            }

            if lua.unlikely_memory_error() {
                ffi::lua_clonefunction(state, -1);
            } else {
                protect_lua!(state, 1, 1, fn(state) ffi::lua_clonefunction(state, -1))?;
            }
            Ok(Function(lua.pop_ref()))
        }
    }
}

struct WrappedFunction(pub(crate) Callback);

#[cfg(feature = "async")]
struct WrappedAsyncFunction(pub(crate) AsyncCallback);

impl Function {
    /// Wraps a Rust function or closure, returning an opaque type that implements [`IntoLua`]
    /// trait.
    #[inline]
    pub fn wrap<F, A, R>(func: F) -> impl IntoLua
    where
        F: LuaNativeFn<A, Output = Result<R>> + MaybeSend + 'static,
        A: FromLuaMulti,
        R: IntoLuaMulti,
    {
        WrappedFunction(Box::new(move |lua, nargs| unsafe {
            let args = A::from_stack_args(nargs, 1, None, lua)?;
            func.call(args)?.push_into_stack_multi(lua)
        }))
    }

    /// Wraps a Rust mutable closure, returning an opaque type that implements [`IntoLua`] trait.
    pub fn wrap_mut<F, A, R>(func: F) -> impl IntoLua
    where
        F: LuaNativeFnMut<A, Output = Result<R>> + MaybeSend + 'static,
        A: FromLuaMulti,
        R: IntoLuaMulti,
    {
        let func = RefCell::new(func);
        WrappedFunction(Box::new(move |lua, nargs| unsafe {
            let mut func = func.try_borrow_mut().map_err(|_| Error::RecursiveMutCallback)?;
            let args = A::from_stack_args(nargs, 1, None, lua)?;
            func.call(args)?.push_into_stack_multi(lua)
        }))
    }

    /// Wraps a Rust function or closure, returning an opaque type that implements [`IntoLua`]
    /// trait.
    ///
    /// This function is similar to [`Function::wrap`] but any returned `Result` will be converted
    /// to a `ok, err` tuple without throwing an exception.
    #[inline]
    pub fn wrap_raw<F, A>(func: F) -> impl IntoLua
    where
        F: LuaNativeFn<A> + MaybeSend + 'static,
        A: FromLuaMulti,
    {
        WrappedFunction(Box::new(move |lua, nargs| unsafe {
            let args = A::from_stack_args(nargs, 1, None, lua)?;
            func.call(args).push_into_stack_multi(lua)
        }))
    }

    /// Wraps a Rust mutable closure, returning an opaque type that implements [`IntoLua`] trait.
    ///
    /// This function is similar to [`Function::wrap_mut`] but any returned `Result` will be
    /// converted to a `ok, err` tuple without throwing an exception.
    #[inline]
    pub fn wrap_raw_mut<F, A>(func: F) -> impl IntoLua
    where
        F: LuaNativeFnMut<A> + MaybeSend + 'static,
        A: FromLuaMulti,
    {
        let func = RefCell::new(func);
        WrappedFunction(Box::new(move |lua, nargs| unsafe {
            let mut func = func.try_borrow_mut().map_err(|_| Error::RecursiveMutCallback)?;
            let args = A::from_stack_args(nargs, 1, None, lua)?;
            func.call(args).push_into_stack_multi(lua)
        }))
    }

    /// Wraps a Rust async function or closure, returning an opaque type that implements [`IntoLua`]
    /// trait.
    #[cfg(feature = "async")]
    #[cfg_attr(docsrs, doc(cfg(feature = "async")))]
    pub fn wrap_async<F, A, R>(func: F) -> impl IntoLua
    where
        F: LuaNativeAsyncFn<A, Output = Result<R>> + MaybeSend + 'static,
        A: FromLuaMulti,
        R: IntoLuaMulti,
    {
        WrappedAsyncFunction(Box::new(move |rawlua, nargs| unsafe {
            let args = match A::from_stack_args(nargs, 1, None, rawlua) {
                Ok(args) => args,
                Err(e) => return Box::pin(future::ready(Err(e))),
            };
            let lua = rawlua.lua();
            let fut = func.call(args);
            Box::pin(async move { fut.await?.push_into_stack_multi(lua.raw_lua()) })
        }))
    }

    /// Wraps a Rust async function or closure, returning an opaque type that implements [`IntoLua`]
    /// trait.
    ///
    /// This function is similar to [`Function::wrap_async`] but any returned `Result` will be
    /// converted to a `ok, err` tuple without throwing an exception.
    #[cfg(feature = "async")]
    #[cfg_attr(docsrs, doc(cfg(feature = "async")))]
    pub fn wrap_raw_async<F, A>(func: F) -> impl IntoLua
    where
        F: LuaNativeAsyncFn<A> + MaybeSend + 'static,
        A: FromLuaMulti,
    {
        WrappedAsyncFunction(Box::new(move |rawlua, nargs| unsafe {
            let args = match A::from_stack_args(nargs, 1, None, rawlua) {
                Ok(args) => args,
                Err(e) => return Box::pin(future::ready(Err(e))),
            };
            let lua = rawlua.lua();
            let fut = func.call(args);
            Box::pin(async move { fut.await.push_into_stack_multi(lua.raw_lua()) })
        }))
    }
}

impl IntoLua for WrappedFunction {
    #[inline]
    fn into_lua(self, lua: &Lua) -> Result<Value> {
        lua.lock().create_callback(self.0).map(Value::Function)
    }
}

#[cfg(feature = "async")]
impl IntoLua for WrappedAsyncFunction {
    #[inline]
    fn into_lua(self, lua: &Lua) -> Result<Value> {
        lua.lock().create_async_callback(self.0).map(Value::Function)
    }
}

impl LuaType for Function {
    const TYPE_ID: c_int = ffi::LUA_TFUNCTION;
}

#[cfg(feature = "async")]
#[must_use = "futures do nothing unless you `.await` or poll them"]
pub struct AsyncCallFuture<R: FromLuaMulti>(Result<AsyncThread<R>>);

#[cfg(feature = "async")]
impl<R: FromLuaMulti> AsyncCallFuture<R> {
    pub(crate) fn error(err: Error) -> Self {
        AsyncCallFuture(Err(err))
    }
}

#[cfg(feature = "async")]
impl<R: FromLuaMulti> Future for AsyncCallFuture<R> {
    type Output = Result<R>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // Safety: We're not moving any pinned data
        let this = unsafe { self.get_unchecked_mut() };
        match &mut this.0 {
            Ok(thread) => {
                let pinned_thread = unsafe { Pin::new_unchecked(thread) };
                pinned_thread.poll(cx)
            }
            Err(err) => Poll::Ready(Err(err.clone())),
        }
    }
}

#[cfg(test)]
mod assertions {
    use super::*;

    #[cfg(not(feature = "send"))]
    static_assertions::assert_not_impl_any!(Function: Send);
    #[cfg(feature = "send")]
    static_assertions::assert_impl_all!(Function: Send, Sync);

    #[cfg(all(feature = "async", feature = "send"))]
    static_assertions::assert_impl_all!(AsyncCallFuture<()>: Send);
}
