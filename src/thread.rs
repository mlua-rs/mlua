use std::os::raw::c_int;

use crate::error::{Error, Result};
use crate::ffi;
use crate::types::LuaRef;
use crate::util::{
    assert_stack, check_stack, error_traceback, pop_error, protect_lua_closure, StackGuard,
};
use crate::value::{FromLuaMulti, MultiValue, ToLuaMulti};

#[cfg(feature = "async")]
use {
    crate::{
        error::ExternalError,
        lua::{AsyncPollPending, Lua, WAKER_REGISTRY_KEY},
        util::{get_gc_userdata, push_gc_userdata},
        value::Value,
    },
    futures_core::{future::Future, stream::Stream},
    std::{
        cell::RefCell,
        marker::PhantomData,
        os::raw::c_void,
        pin::Pin,
        task::{Context, Poll, Waker},
    },
};

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

/// Thread (coroutine) representation as an async [`Future`] or [`Stream`].
///
/// Requires `feature = "async"`
///
/// [`Future`]: ../futures_core/future/trait.Future.html
/// [`Stream`]: ../futures_core/stream/trait.Stream.html
#[cfg(feature = "async")]
#[cfg_attr(docsrs, doc(cfg(feature = "async")))]
#[derive(Debug)]
pub struct AsyncThread<'lua, R> {
    thread: Thread<'lua>,
    args0: RefCell<Option<Result<MultiValue<'lua>>>>,
    ret: PhantomData<R>,
}

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
    /// # use mlua::{Error, Lua, Result, Thread};
    /// # fn main() -> Result<()> {
    /// # let lua = Lua::new();
    /// let thread: Thread = lua.load(r#"
    ///     coroutine.create(function(arg)
    ///         assert(arg == 42)
    ///         local yieldarg = coroutine.yield(123)
    ///         assert(yieldarg == 43)
    ///         return 987
    ///     end)
    /// "#).eval()?;
    ///
    /// assert_eq!(thread.resume::<_, u32>(42)?, 123);
    /// assert_eq!(thread.resume::<_, u32>(43)?, 987);
    ///
    /// // The coroutine has now returned, so `resume` will fail
    /// match thread.resume::<_, u32>(()) {
    ///     Err(Error::CoroutineInactive) => {},
    ///     unexpected => panic!("unexpected result {:?}", unexpected),
    /// }
    /// # Ok(())
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

            let mut nresults = 0;

            let ret = ffi::lua_resume(thread_state, lua.state, nargs, &mut nresults as *mut c_int);
            if ret != ffi::LUA_OK && ret != ffi::LUA_YIELD {
                protect_lua_closure(lua.state, 0, 0, |_| {
                    error_traceback(thread_state);
                    0
                })?;
                return Err(pop_error(thread_state, ret));
            }

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

    /// Converts Thread to an AsyncThread which implements Future and Stream traits.
    ///
    /// `args` are passed as arguments to the thread function for first call.
    /// The object call `resume()` while polling and also allows to run rust futures
    /// to completion using an executor.
    ///
    /// Using AsyncThread as a Stream allows to iterate through `coroutine.yield()`
    /// values whereas Future version discards that values and poll until the final
    /// one (returned from the thread function).
    ///
    /// Requires `feature = "async"`
    ///
    /// # Examples
    ///
    /// ```
    /// # use mlua::{Lua, Result, Thread};
    /// use futures::stream::TryStreamExt;
    /// # #[tokio::main]
    /// # async fn main() -> Result<()> {
    /// # let lua = Lua::new();
    /// let thread: Thread = lua.load(r#"
    ///     coroutine.create(function (sum)
    ///         for i = 1,10 do
    ///             sum = sum + i
    ///             coroutine.yield(sum)
    ///         end
    ///         return sum
    ///     end)
    /// "#).eval()?;
    ///
    /// let mut stream = thread.into_async::<_, i64>(1);
    /// let mut sum = 0;
    /// while let Some(n) = stream.try_next().await? {
    ///     sum += n;
    /// }
    ///
    /// assert_eq!(sum, 286);
    ///
    /// # Ok(())
    /// # }
    /// ```
    #[cfg(feature = "async")]
    #[cfg_attr(docsrs, doc(cfg(feature = "async")))]
    pub fn into_async<A, R>(self, args: A) -> AsyncThread<'lua, R>
    where
        A: ToLuaMulti<'lua>,
        R: FromLuaMulti<'lua>,
    {
        let args = args.to_lua_multi(&self.0.lua);
        AsyncThread {
            thread: self,
            args0: RefCell::new(Some(args)),
            ret: PhantomData,
        }
    }
}

impl<'lua> PartialEq for Thread<'lua> {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

#[cfg(feature = "async")]
impl<'lua, R> Stream for AsyncThread<'lua, R>
where
    R: FromLuaMulti<'lua>,
{
    type Item = Result<R>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let lua = self.thread.0.lua;

        match self.thread.status() {
            ThreadStatus::Resumable => {}
            _ => return Poll::Ready(None),
        };

        let _wg = WakerGuard::new(lua.state, cx.waker().clone());
        let ret: MultiValue = if let Some(args) = self.args0.borrow_mut().take() {
            self.thread.resume(args?)?
        } else {
            self.thread.resume(())?
        };

        if is_poll_pending(lua, &ret) {
            return Poll::Pending;
        }

        cx.waker().wake_by_ref();
        Poll::Ready(Some(R::from_lua_multi(ret, lua)))
    }
}

#[cfg(feature = "async")]
impl<'lua, R> Future for AsyncThread<'lua, R>
where
    R: FromLuaMulti<'lua>,
{
    type Output = Result<R>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let lua = self.thread.0.lua;

        match self.thread.status() {
            ThreadStatus::Resumable => {}
            _ => return Poll::Ready(Err("Thread already finished".to_lua_err())),
        };

        let _wg = WakerGuard::new(lua.state, cx.waker().clone());
        let ret: MultiValue = if let Some(args) = self.args0.borrow_mut().take() {
            self.thread.resume(args?)?
        } else {
            self.thread.resume(())?
        };

        if is_poll_pending(lua, &ret) {
            return Poll::Pending;
        }

        if let ThreadStatus::Resumable = self.thread.status() {
            // Ignore value returned via yield()
            cx.waker().wake_by_ref();
            return Poll::Pending;
        }

        Poll::Ready(R::from_lua_multi(ret, lua))
    }
}

#[cfg(feature = "async")]
fn is_poll_pending(lua: &Lua, val: &MultiValue) -> bool {
    if val.len() != 1 {
        return false;
    }

    if let Some(Value::UserData(ud)) = val.iter().next() {
        unsafe {
            let _sg = StackGuard::new(lua.state);
            assert_stack(lua.state, 3);

            lua.push_ref(&ud.0);
            let is_pending = get_gc_userdata::<AsyncPollPending>(lua.state, -1)
                .as_ref()
                .is_some();
            ffi::lua_pop(lua.state, 1);

            return is_pending;
        }
    }

    false
}

#[cfg(feature = "async")]
struct WakerGuard(*mut ffi::lua_State);

#[cfg(feature = "async")]
impl WakerGuard {
    pub fn new(state: *mut ffi::lua_State, waker: Waker) -> Result<WakerGuard> {
        unsafe {
            let _sg = StackGuard::new(state);
            assert_stack(state, 6);

            ffi::lua_pushlightuserdata(state, &WAKER_REGISTRY_KEY as *const u8 as *mut c_void);
            push_gc_userdata(state, waker)?;
            ffi::lua_rawset(state, ffi::LUA_REGISTRYINDEX);

            Ok(WakerGuard(state))
        }
    }
}

#[cfg(feature = "async")]
impl Drop for WakerGuard {
    fn drop(&mut self) {
        unsafe {
            let state = self.0;
            let _sg = StackGuard::new(state);
            assert_stack(state, 2);

            ffi::lua_pushlightuserdata(state, &WAKER_REGISTRY_KEY as *const u8 as *mut c_void);
            ffi::lua_pushnil(state);
            ffi::lua_rawset(state, ffi::LUA_REGISTRYINDEX);
        }
    }
}
