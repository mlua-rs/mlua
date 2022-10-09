use std::mem;
use std::os::raw::{c_int, c_void};
use std::ptr;
use std::slice;

use crate::error::{Error, Result};
use crate::ffi;
use crate::types::LuaRef;
use crate::util::{
    assert_stack, check_stack, error_traceback, pop_error, ptr_to_cstr_bytes, StackGuard,
};
use crate::value::{FromLuaMulti, ToLuaMulti};

#[cfg(feature = "async")]
use {futures_core::future::LocalBoxFuture, futures_util::future};

/// Handle to an internal Lua function.
#[derive(Clone, Debug)]
pub struct Function<'lua>(pub(crate) LuaRef<'lua>);

#[derive(Clone, Debug)]
pub struct FunctionInfo {
    pub name: Option<Vec<u8>>,
    pub name_what: Option<Vec<u8>>,
    pub what: Option<Vec<u8>>,
    pub source: Option<Vec<u8>>,
    pub short_src: Option<Vec<u8>>,
    pub line_defined: i32,
    #[cfg(not(feature = "luau"))]
    pub last_line_defined: i32,
}

/// Luau function coverage snapshot.
#[cfg(any(feature = "luau", doc))]
#[cfg_attr(docsrs, doc(cfg(feature = "luau")))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CoverageInfo {
    pub function: Option<std::string::String>,
    pub line_defined: i32,
    pub depth: i32,
    pub hits: Vec<i32>,
}

impl<'lua> Function<'lua> {
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
    /// assert_eq!(tostring.call::<_, String>(123)?, "123");
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
    /// assert_eq!(sum.call::<_, u32>((3, 4))?, 3 + 4);
    ///
    /// # Ok(())
    /// # }
    /// ```
    pub fn call<A: ToLuaMulti<'lua>, R: FromLuaMulti<'lua>>(&self, args: A) -> Result<R> {
        let lua = self.0.lua;

        let mut args = args.to_lua_multi(lua)?;
        let nargs = args.len() as c_int;

        let results = unsafe {
            let _sg = StackGuard::new(lua.state);
            check_stack(lua.state, nargs + 3)?;

            ffi::lua_pushcfunction(lua.state, error_traceback);
            let stack_start = ffi::lua_gettop(lua.state);
            lua.push_ref(&self.0);
            for arg in args.drain_all() {
                lua.push_value(arg)?;
            }
            let ret = ffi::lua_pcall(lua.state, nargs, ffi::LUA_MULTRET, stack_start);
            if ret != ffi::LUA_OK {
                return Err(pop_error(lua.state, ret));
            }
            let nresults = ffi::lua_gettop(lua.state) - stack_start;
            let mut results = args; // Reuse MultiValue container
            assert_stack(lua.state, 2);
            for _ in 0..nresults {
                results.push_front(lua.pop_value());
            }
            ffi::lua_pop(lua.state, 1);
            results
        };
        R::from_lua_multi(results, lua)
    }

    /// Returns a future that, when polled, calls `self`, passing `args` as function arguments,
    /// and drives the execution.
    ///
    /// Internally it wraps the function to an [`AsyncThread`].
    ///
    /// Requires `feature = "async"`
    ///
    /// # Examples
    ///
    /// ```
    /// use std::time::Duration;
    /// use futures_timer::Delay;
    /// # use mlua::{Lua, Result};
    /// # #[tokio::main]
    /// # async fn main() -> Result<()> {
    /// # let lua = Lua::new();
    ///
    /// let sleep = lua.create_async_function(move |_lua, n: u64| async move {
    ///     Delay::new(Duration::from_millis(n)).await;
    ///     Ok(())
    /// })?;
    ///
    /// sleep.call_async(10).await?;
    ///
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// [`AsyncThread`]: crate::AsyncThread
    #[cfg(feature = "async")]
    #[cfg_attr(docsrs, doc(cfg(feature = "async")))]
    pub fn call_async<'fut, A, R>(&self, args: A) -> LocalBoxFuture<'fut, Result<R>>
    where
        'lua: 'fut,
        A: ToLuaMulti<'lua>,
        R: FromLuaMulti<'lua> + 'fut,
    {
        let lua = self.0.lua;
        match lua.create_recycled_thread(self.clone()) {
            Ok(t) => {
                let mut t = t.into_async(args);
                t.set_recyclable(true);
                Box::pin(t)
            }
            Err(e) => Box::pin(future::err(e)),
        }
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
    /// assert_eq!(bound_a.call::<_, u32>(2)?, 1 + 2);
    ///
    /// let bound_a_and_b = sum.bind(13)?.bind(57)?;
    /// assert_eq!(bound_a_and_b.call::<_, u32>(())?, 13 + 57);
    ///
    /// # Ok(())
    /// # }
    /// ```
    pub fn bind<A: ToLuaMulti<'lua>>(&self, args: A) -> Result<Function<'lua>> {
        unsafe extern "C" fn args_wrapper_impl(state: *mut ffi::lua_State) -> c_int {
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

        let lua = self.0.lua;

        let args = args.to_lua_multi(lua)?;
        let nargs = args.len() as c_int;

        if nargs == 0 {
            return Ok(self.clone());
        }

        if nargs + 1 > ffi::LUA_MAX_UPVALUES {
            return Err(Error::BindError);
        }

        let args_wrapper = unsafe {
            let _sg = StackGuard::new(lua.state);
            check_stack(lua.state, nargs + 3)?;

            ffi::lua_pushinteger(lua.state, nargs as ffi::lua_Integer);
            for arg in args {
                lua.push_value(arg)?;
            }
            protect_lua!(lua.state, nargs + 1, 1, fn(state) {
                ffi::lua_pushcclosure(state, args_wrapper_impl, ffi::lua_gettop(state));
            })?;

            Function(lua.pop_ref())
        };

        lua.load(
            r#"
            local func, args_wrapper = ...
            return function(...)
                return func(args_wrapper(...))
            end
            "#,
        )
        .try_cache()
        .set_name("_mlua_bind")?
        .call((self.clone(), args_wrapper))
    }

    /// Returns information about the function.
    ///
    /// Corresponds to the `>Sn` what mask for [`lua_getinfo`] when applied to the function.
    ///
    /// [`lua_getinfo`]: https://www.lua.org/manual/5.4/manual.html#lua_getinfo
    pub fn info(&self) -> FunctionInfo {
        let lua = self.0.lua;
        unsafe {
            let _sg = StackGuard::new(lua.state);
            assert_stack(lua.state, 1);

            let mut ar: ffi::lua_Debug = mem::zeroed();
            lua.push_ref(&self.0);
            #[cfg(not(feature = "luau"))]
            let res = ffi::lua_getinfo(lua.state, cstr!(">Sn"), &mut ar);
            #[cfg(feature = "luau")]
            let res = ffi::lua_getinfo(lua.state, -1, cstr!("sn"), &mut ar);
            mlua_assert!(res != 0, "lua_getinfo failed with `>Sn`");

            FunctionInfo {
                name: ptr_to_cstr_bytes(ar.name).map(|s| s.to_vec()),
                #[cfg(not(feature = "luau"))]
                name_what: ptr_to_cstr_bytes(ar.namewhat).map(|s| s.to_vec()),
                #[cfg(feature = "luau")]
                name_what: None,
                what: ptr_to_cstr_bytes(ar.what).map(|s| s.to_vec()),
                source: ptr_to_cstr_bytes(ar.source).map(|s| s.to_vec()),
                #[cfg(not(feature = "luau"))]
                short_src: ptr_to_cstr_bytes(ar.short_src.as_ptr()).map(|s| s.to_vec()),
                #[cfg(feature = "luau")]
                short_src: ptr_to_cstr_bytes(ar.short_src).map(|s| s.to_vec()),
                line_defined: ar.linedefined,
                #[cfg(not(feature = "luau"))]
                last_line_defined: ar.lastlinedefined,
            }
        }
    }

    /// Dumps the function as a binary chunk.
    ///
    /// If `strip` is true, the binary representation may not include all debug information
    /// about the function, to save space.
    ///
    /// For Luau a [Compiler] can be used to compile Lua chunks to bytecode.
    ///
    /// [Compiler]: crate::chunk::Compiler
    #[cfg(not(feature = "luau"))]
    #[cfg_attr(docsrs, doc(cfg(not(feature = "luau"))))]
    pub fn dump(&self, strip: bool) -> Vec<u8> {
        unsafe extern "C" fn writer(
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

        let lua = self.0.lua;
        let mut data: Vec<u8> = Vec::new();
        unsafe {
            let _sg = StackGuard::new(lua.state);
            assert_stack(lua.state, 1);

            lua.push_ref(&self.0);
            let data_ptr = &mut data as *mut Vec<u8> as *mut c_void;
            ffi::lua_dump(lua.state, writer, data_ptr, strip as i32);
            ffi::lua_pop(lua.state, 1);
        }

        data
    }

    /// Retrieves recorded coverage information about this Lua function including inner calls.
    ///
    /// This function takes a callback as an argument and calls it providing [`CoverageInfo`] snapshot
    /// per each executed inner function.
    ///
    /// Recording of coverage information is controlled by [`Compiler::set_coverage_level`] option.
    ///
    /// Requires `feature = "luau"`
    ///
    /// [`Compiler::set_coverage_level`]: crate::chunk::Compiler::set_coverage_level
    #[cfg(any(feature = "luau", docsrs))]
    #[cfg_attr(docsrs, doc(cfg(feature = "luau")))]
    pub fn coverage<F>(&self, mut func: F)
    where
        F: FnMut(CoverageInfo),
    {
        use std::ffi::CStr;
        use std::os::raw::c_char;

        unsafe extern "C" fn callback<F: FnMut(CoverageInfo)>(
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
            let rust_callback = &mut *(data as *mut F);
            rust_callback(CoverageInfo {
                function,
                line_defined,
                depth,
                hits: slice::from_raw_parts(hits, size).to_vec(),
            });
        }

        let lua = self.0.lua;
        unsafe {
            let _sg = StackGuard::new(lua.state);
            assert_stack(lua.state, 1);

            lua.push_ref(&self.0);
            let func_ptr = &mut func as *mut F as *mut c_void;
            ffi::lua_getcoverage(lua.state, -1, func_ptr, callback::<F>);
        }
    }
}

impl<'lua> PartialEq for Function<'lua> {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}
