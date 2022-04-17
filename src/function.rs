use std::ffi::CStr;
use std::os::raw::c_int;
use std::ptr;

use crate::error::{Error, Result};
use crate::ffi;
use crate::types::LuaRef;
use crate::util::{assert_stack, check_stack, error_traceback, pop_error, StackGuard};
use crate::value::{FromLuaMulti, ToLuaMulti};

#[cfg(feature = "async")]
use {futures_core::future::LocalBoxFuture, futures_util::future};

/// Handle to an internal Lua function.
#[derive(Clone, Debug)]
pub struct Function<'lua>(pub(crate) LuaRef<'lua>);

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

    /// Returns a Feature that, when polled, calls `self`, passing `args` as function arguments,
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
        unsafe extern "C" fn bind_call_impl(state: *mut ffi::lua_State) -> c_int {
            let nargs = ffi::lua_gettop(state);
            let nbinds = ffi::lua_tointeger(state, ffi::lua_upvalueindex(2)) as c_int;
            ffi::luaL_checkstack(state, nbinds + 2, ptr::null());

            ffi::lua_settop(state, nargs + nbinds + 1);
            ffi::lua_rotate(state, -(nargs + nbinds + 1), nbinds + 1);

            ffi::lua_pushvalue(state, ffi::lua_upvalueindex(1));
            ffi::lua_replace(state, 1);

            for i in 0..nbinds {
                ffi::lua_pushvalue(state, ffi::lua_upvalueindex(i + 3));
                ffi::lua_replace(state, i + 2);
            }

            ffi::lua_call(state, nargs + nbinds, ffi::LUA_MULTRET);
            ffi::lua_gettop(state)
        }

        let lua = self.0.lua;

        let args = args.to_lua_multi(lua)?;
        let nargs = args.len() as c_int;

        if nargs + 2 > ffi::LUA_MAX_UPVALUES {
            return Err(Error::BindError);
        }

        unsafe {
            let _sg = StackGuard::new(lua.state);
            check_stack(lua.state, nargs + 5)?;

            lua.push_ref(&self.0);
            ffi::lua_pushinteger(lua.state, nargs as ffi::lua_Integer);
            for arg in args {
                lua.push_value(arg)?;
            }
            protect_lua!(lua.state, nargs + 2, 1, fn(state) {
                ffi::lua_pushcclosure(state, bind_call_impl, ffi::lua_gettop(state));
            })?;

            Ok(Function(lua.pop_ref()))
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
        use std::os::raw::c_void;
        use std::slice;

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
            let strip = if strip { 1 } else { 0 };
            ffi::lua_dump(lua.state, writer, data_ptr, strip);
            ffi::lua_pop(lua.state, 1);
        }

        data
    }

    /// Returns source and line number information about the function.
    ///
    /// Corresponds to the `>S` what mask for `lua_getinfo` when applied
    /// to the function.
    #[cfg(not(feature = "luau"))]
    pub fn info(&self) -> FunctionInfo {
        use std::os::raw::c_char;

        let lua = self.0.lua;
        unsafe {
            let mut ar: ffi::lua_Debug = std::mem::zeroed();
            lua.push_ref(&self.0);
            mlua_assert!(
                ffi::lua_getinfo(lua.state, cstr!(">S"), &mut ar as *mut ffi::lua_Debug) != 0,
                "lua_getinfo failed with `>S`"
            );
            FunctionInfo {
                source: CStr::from_ptr(ar.source).to_bytes().to_vec(),
                short_src: CStr::from_ptr(&ar.short_src as *const c_char)
                    .to_bytes()
                    .to_vec(),
                line_defined: ar.linedefined as i32,
                last_line_defined: ar.lastlinedefined as i32,
                what: String::from_utf8_lossy(CStr::from_ptr(ar.what as *const c_char).to_bytes())
                    .into_owned(),
            }
        }
    }
}

impl<'lua> PartialEq for Function<'lua> {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct FunctionInfo {
    pub source: Vec<u8>,
    pub short_src: Vec<u8>,
    pub line_defined: i32,
    pub last_line_defined: i32,
    pub what: String,
}
