use std::os::raw::c_int;

use crate::error::{Error, Result};
use crate::ffi;
use crate::types::LuaRef;
use crate::util::{
    assert_stack, check_stack, error_traceback, pop_error, protect_lua_closure, StackGuard,
};
use crate::value::{FromLuaMulti, MultiValue, ToLuaMulti};

/// Status of a Lua thread (or coroutine).
#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum ThreadStatus {
    /// The thread was just created, or is suspended because it has called `coroutine.yield`.
    ///
    /// If a thread is in this state, it can be resumed by calling [`Thread::resume`].
    ///
    /// [`Thread::resume`]: struct.Thread.html#method.resume
    Resumable,
    /// Either the thread has finished executing, or the thread is currently running.
    Unresumable,
    /// The thread has raised a Lua error during execution.
    Error,
}

/// Handle to an internal Lua thread (or coroutine).
#[derive(Clone, Debug)]
pub struct Thread<'lua>(pub(crate) LuaRef<'lua>);

impl<'lua> Thread<'lua> {
    /// Resumes execution of this thread.
    ///
    /// Equivalent to `coroutine.resume`.
    ///
    /// Passes `args` as arguments to the thread. If the coroutine has called `coroutine.yield`, it
    /// will return these arguments. Otherwise, the coroutine wasn't yet started, so the arguments
    /// are passed to its main function.
    ///
    /// If the thread is no longer in `Active` state (meaning it has finished execution or
    /// encountered an error), this will return `Err(CoroutineInactive)`, otherwise will return `Ok`
    /// as follows:
    ///
    /// If the thread calls `coroutine.yield`, returns the values passed to `yield`. If the thread
    /// `return`s values from its main function, returns those.
    ///
    /// # Examples
    ///
    /// ```
    /// # use mlua::{Lua, Thread, Error};
    /// # fn main() -> Result<()> {
    /// let lua = Lua::new();
    /// let thread: Thread = lua.eval(r#"
    ///     coroutine.create(function(arg)
    ///         assert(arg == 42)
    ///         local yieldarg = coroutine.yield(123)
    ///         assert(yieldarg == 43)
    ///         return 987
    ///     end)
    /// "#, None).unwrap();
    ///
    /// assert_eq!(thread.resume::<_, u32>(42).unwrap(), 123);
    /// assert_eq!(thread.resume::<_, u32>(43).unwrap(), 987);
    ///
    /// // The coroutine has now returned, so `resume` will fail
    /// match thread.resume::<_, u32>(()) {
    ///     Err(Error::CoroutineInactive) => {},
    ///     unexpected => panic!("unexpected result {:?}", unexpected),
    /// }
    /// # }
    /// ```
    pub fn resume<A, R>(&self, args: A) -> Result<R>
    where
        A: ToLuaMulti<'lua>,
        R: FromLuaMulti<'lua>,
    {
        let lua = self.0.lua;
        let args = args.to_lua_multi(lua)?;
        let results = unsafe {
            let _sg = StackGuard::new(lua.state);
            assert_stack(lua.state, 3);

            lua.push_ref(&self.0);
            let thread_state = ffi::lua_tothread(lua.state, -1);

            let status = ffi::lua_status(thread_state);
            if status != ffi::LUA_YIELD && ffi::lua_gettop(thread_state) == 0 {
                return Err(Error::CoroutineInactive);
            }

            ffi::lua_pop(lua.state, 1);

            let nargs = args.len() as c_int;
            check_stack(lua.state, nargs)?;
            check_stack(thread_state, nargs + 1)?;

            for arg in args {
                lua.push_value(arg)?;
            }
            ffi::lua_xmove(lua.state, thread_state, nargs);

            let ret = ffi::lua_resume(thread_state, lua.state, nargs);
            if ret != ffi::LUA_OK && ret != ffi::LUA_YIELD {
                protect_lua_closure(lua.state, 0, 0, |_| {
                    error_traceback(thread_state);
                    0
                })?;
                return Err(pop_error(thread_state, ret));
            }

            let nresults = ffi::lua_gettop(thread_state);
            let mut results = MultiValue::new();
            ffi::lua_xmove(thread_state, lua.state, nresults);

            assert_stack(lua.state, 2);
            for _ in 0..nresults {
                results.push_front(lua.pop_value());
            }
            results
        };
        R::from_lua_multi(results, lua)
    }

    /// Gets the status of the thread.
    pub fn status(&self) -> ThreadStatus {
        let lua = self.0.lua;
        unsafe {
            let _sg = StackGuard::new(lua.state);
            assert_stack(lua.state, 1);

            lua.push_ref(&self.0);
            let thread_state = ffi::lua_tothread(lua.state, -1);
            ffi::lua_pop(lua.state, 1);

            let status = ffi::lua_status(thread_state);
            if status != ffi::LUA_OK && status != ffi::LUA_YIELD {
                ThreadStatus::Error
            } else if status == ffi::LUA_YIELD || ffi::lua_gettop(thread_state) > 0 {
                ThreadStatus::Resumable
            } else {
                ThreadStatus::Unresumable
            }
        }
    }
}
