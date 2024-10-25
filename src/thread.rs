use std::fmt;
use std::os::raw::{c_int, c_void};

use crate::error::{Error, Result};
#[allow(unused)]
use crate::state::Lua;
use crate::state::RawLua;
use crate::traits::{FromLuaMulti, IntoLuaMulti};
use crate::types::{LuaType, ValueRef};
use crate::util::{check_stack, error_traceback_thread, pop_error, StackGuard};

#[cfg(not(feature = "luau"))]
use crate::{
    hook::{Debug, HookTriggers},
    types::MaybeSend,
};

#[cfg(feature = "async")]
use {
    futures_util::stream::Stream,
    std::{
        future::Future,
        marker::PhantomData,
        pin::Pin,
        ptr::NonNull,
        task::{Context, Poll, Waker},
    },
};

/// Status of a Lua thread (coroutine).
#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum ThreadStatus {
    /// The thread was just created or is suspended (yielded).
    ///
    /// If a thread is in this state, it can be resumed by calling [`Thread::resume`].
    Resumable,
    /// The thread is currently running.
    Running,
    /// The thread has finished executing.
    Finished,
    /// The thread has raised a Lua error during execution.
    Error,
}

/// Handle to an internal Lua thread (coroutine).
#[derive(Clone)]
pub struct Thread(pub(crate) ValueRef, pub(crate) *mut ffi::lua_State);

#[cfg(feature = "send")]
unsafe impl Send for Thread {}
#[cfg(feature = "send")]
unsafe impl Sync for Thread {}

/// Thread (coroutine) representation as an async [`Future`] or [`Stream`].
///
/// Requires `feature = "async"`
///
/// [`Future`]: std::future::Future
/// [`Stream`]: futures_util::stream::Stream
#[cfg(feature = "async")]
#[cfg_attr(docsrs, doc(cfg(feature = "async")))]
#[must_use = "futures do nothing unless you `.await` or poll them"]
pub struct AsyncThread<A, R> {
    thread: Thread,
    init_args: Option<A>,
    ret: PhantomData<R>,
    recycle: bool,
}

impl Thread {
    #[inline(always)]
    fn state(&self) -> *mut ffi::lua_State {
        self.1
    }

    /// Resumes execution of this thread.
    ///
    /// Equivalent to [`coroutine.resume`].
    ///
    /// Passes `args` as arguments to the thread. If the coroutine has called [`coroutine.yield`],
    /// it will return these arguments. Otherwise, the coroutine wasn't yet started, so the
    /// arguments are passed to its main function.
    ///
    /// If the thread is no longer resumable (meaning it has finished execution or encountered an
    /// error), this will return [`Error::CoroutineUnresumable`], otherwise will return `Ok` as
    /// follows:
    ///
    /// If the thread calls [`coroutine.yield`], returns the values passed to `yield`. If the thread
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
    /// assert_eq!(thread.resume::<u32>(42)?, 123);
    /// assert_eq!(thread.resume::<u32>(43)?, 987);
    ///
    /// // The coroutine has now returned, so `resume` will fail
    /// match thread.resume::<u32>(()) {
    ///     Err(Error::CoroutineUnresumable) => {},
    ///     unexpected => panic!("unexpected result {:?}", unexpected),
    /// }
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// [`coroutine.resume`]: https://www.lua.org/manual/5.4/manual.html#pdf-coroutine.resume
    /// [`coroutine.yield`]: https://www.lua.org/manual/5.4/manual.html#pdf-coroutine.yield
    pub fn resume<R>(&self, args: impl IntoLuaMulti) -> Result<R>
    where
        R: FromLuaMulti,
    {
        let lua = self.0.lua.lock();
        if self.status_inner(&lua) != ThreadStatus::Resumable {
            return Err(Error::CoroutineUnresumable);
        }

        let state = lua.state();
        let thread_state = self.state();
        unsafe {
            let _sg = StackGuard::new(state);
            let _thread_sg = StackGuard::with_top(thread_state, 0);

            let nresults = self.resume_inner(&lua, args)?;
            check_stack(state, nresults + 1)?;
            ffi::lua_xmove(thread_state, state, nresults);

            R::from_stack_multi(nresults, &lua)
        }
    }

    /// Resumes execution of this thread.
    ///
    /// It's similar to `resume()` but leaves `nresults` values on the thread stack.
    unsafe fn resume_inner(&self, lua: &RawLua, args: impl IntoLuaMulti) -> Result<c_int> {
        let state = lua.state();
        let thread_state = self.state();

        let nargs = args.push_into_stack_multi(lua)?;
        if nargs > 0 {
            check_stack(thread_state, nargs)?;
            ffi::lua_xmove(state, thread_state, nargs);
        }

        let mut nresults = 0;
        let ret = ffi::lua_resume(thread_state, state, nargs, &mut nresults as *mut c_int);
        if ret != ffi::LUA_OK && ret != ffi::LUA_YIELD {
            if ret == ffi::LUA_ERRMEM {
                // Don't call error handler for memory errors
                return Err(pop_error(thread_state, ret));
            }
            check_stack(state, 3)?;
            protect_lua!(state, 0, 1, |state| error_traceback_thread(state, thread_state))?;
            return Err(pop_error(state, ret));
        }

        Ok(nresults)
    }

    /// Gets the status of the thread.
    pub fn status(&self) -> ThreadStatus {
        self.status_inner(&self.0.lua.lock())
    }

    /// Gets the status of the thread (internal implementation).
    pub(crate) fn status_inner(&self, lua: &RawLua) -> ThreadStatus {
        let thread_state = self.state();
        if thread_state == lua.state() {
            // The thread is currently running
            return ThreadStatus::Running;
        }
        let status = unsafe { ffi::lua_status(thread_state) };
        if status != ffi::LUA_OK && status != ffi::LUA_YIELD {
            ThreadStatus::Error
        } else if status == ffi::LUA_YIELD || unsafe { ffi::lua_gettop(thread_state) > 0 } {
            ThreadStatus::Resumable
        } else {
            ThreadStatus::Finished
        }
    }

    /// Sets a hook function that will periodically be called as Lua code executes.
    ///
    /// This function is similar or [`Lua::set_hook`] except that it sets for the thread.
    /// To remove a hook call [`Lua::remove_hook`].
    #[cfg(not(feature = "luau"))]
    #[cfg_attr(docsrs, doc(cfg(not(feature = "luau"))))]
    pub fn set_hook<F>(&self, triggers: HookTriggers, callback: F)
    where
        F: Fn(&Lua, Debug) -> Result<crate::VmState> + MaybeSend + 'static,
    {
        let lua = self.0.lua.lock();
        unsafe {
            lua.set_thread_hook(self.state(), triggers, callback);
        }
    }

    /// Resets a thread
    ///
    /// In [Lua 5.4]: cleans its call stack and closes all pending to-be-closed variables.
    /// Returns a error in case of either the original error that stopped the thread or errors
    /// in closing methods.
    ///
    /// In Luau: resets to the initial state of a newly created Lua thread.
    /// Lua threads in arbitrary states (like yielded or errored) can be reset properly.
    ///
    /// Sets a Lua function for the thread afterwards.
    ///
    /// Requires `feature = "lua54"` OR `feature = "luau"`.
    ///
    /// [Lua 5.4]: https://www.lua.org/manual/5.4/manual.html#lua_closethread
    #[cfg(any(feature = "lua54", feature = "luau"))]
    #[cfg_attr(docsrs, doc(cfg(any(feature = "lua54", feature = "luau"))))]
    pub fn reset(&self, func: crate::function::Function) -> Result<()> {
        let lua = self.0.lua.lock();
        if self.status_inner(&lua) == ThreadStatus::Running {
            return Err(Error::runtime("cannot reset a running thread"));
        }

        let thread_state = self.state();
        unsafe {
            #[cfg(all(feature = "lua54", not(feature = "vendored")))]
            let status = ffi::lua_resetthread(thread_state);
            #[cfg(all(feature = "lua54", feature = "vendored"))]
            let status = ffi::lua_closethread(thread_state, lua.state());
            #[cfg(feature = "lua54")]
            if status != ffi::LUA_OK {
                return Err(pop_error(thread_state, status));
            }
            #[cfg(feature = "luau")]
            ffi::lua_resetthread(thread_state);

            // Push function to the top of the thread stack
            ffi::lua_xpush(lua.ref_thread(), thread_state, func.0.index);

            #[cfg(feature = "luau")]
            {
                // Inherit `LUA_GLOBALSINDEX` from the main thread
                ffi::lua_xpush(lua.main_state(), thread_state, ffi::LUA_GLOBALSINDEX);
                ffi::lua_replace(thread_state, ffi::LUA_GLOBALSINDEX);
            }

            Ok(())
        }
    }

    /// Converts [`Thread`] to an [`AsyncThread`] which implements [`Future`] and [`Stream`] traits.
    ///
    /// `args` are passed as arguments to the thread function for first call.
    /// The object calls [`resume`] while polling and also allow to run Rust futures
    /// to completion using an executor.
    ///
    /// Using [`AsyncThread`] as a [`Stream`] allow to iterate through [`coroutine.yield`]
    /// values whereas [`Future`] version discards that values and poll until the final
    /// one (returned from the thread function).
    ///
    /// Requires `feature = "async"`
    ///
    /// [`Future`]: std::future::Future
    /// [`Stream`]: futures_util::stream::Stream
    /// [`resume`]: https://www.lua.org/manual/5.4/manual.html#lua_resume
    /// [`coroutine.yield`]: https://www.lua.org/manual/5.4/manual.html#pdf-coroutine.yield
    ///
    /// # Examples
    ///
    /// ```
    /// # use mlua::{Lua, Result, Thread};
    /// use futures_util::stream::TryStreamExt;
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
    /// let mut stream = thread.into_async::<i64>(1);
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
    pub fn into_async<R>(self, args: impl IntoLuaMulti) -> AsyncThread<impl IntoLuaMulti, R>
    where
        R: FromLuaMulti,
    {
        AsyncThread {
            thread: self,
            init_args: Some(args),
            ret: PhantomData,
            recycle: false,
        }
    }

    /// Enables sandbox mode on this thread.
    ///
    /// Under the hood replaces the global environment table with a new table,
    /// that performs writes locally and proxies reads to caller's global environment.
    ///
    /// This mode ideally should be used together with the global sandbox mode [`Lua::sandbox`].
    ///
    /// Please note that Luau links environment table with chunk when loading it into Lua state.
    /// Therefore you need to load chunks into a thread to link with the thread environment.
    ///
    /// # Examples
    ///
    /// ```
    /// # use mlua::{Lua, Result};
    /// # #[cfg(feature = "luau")]
    /// # fn main() -> Result<()> {
    /// let lua = Lua::new();
    /// let thread = lua.create_thread(lua.create_function(|lua2, ()| {
    ///     lua2.load("var = 123").exec()?;
    ///     assert_eq!(lua2.globals().get::<u32>("var")?, 123);
    ///     Ok(())
    /// })?)?;
    /// thread.sandbox()?;
    /// thread.resume(())?;
    ///
    /// // The global environment should be unchanged
    /// assert_eq!(lua.globals().get::<Option<u32>>("var")?, None);
    /// # Ok(())
    /// # }
    ///
    /// # #[cfg(not(feature = "luau"))]
    /// # fn main() { }
    /// ```
    ///
    /// Requires `feature = "luau"`
    #[cfg(any(feature = "luau", doc))]
    #[cfg_attr(docsrs, doc(cfg(feature = "luau")))]
    #[doc(hidden)]
    pub fn sandbox(&self) -> Result<()> {
        let lua = self.0.lua.lock();
        let state = lua.state();
        let thread_state = self.state();
        unsafe {
            check_stack(thread_state, 3)?;
            check_stack(state, 3)?;
            protect_lua!(state, 0, 0, |_| ffi::luaL_sandboxthread(thread_state))
        }
    }

    /// Converts this thread to a generic C pointer.
    ///
    /// There is no way to convert the pointer back to its original value.
    ///
    /// Typically this function is used only for hashing and debug information.
    #[inline]
    pub fn to_pointer(&self) -> *const c_void {
        self.0.to_pointer()
    }
}

impl fmt::Debug for Thread {
    fn fmt(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt.debug_tuple("Thread").field(&self.0).finish()
    }
}

impl PartialEq for Thread {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

impl LuaType for Thread {
    const TYPE_ID: c_int = ffi::LUA_TTHREAD;
}

#[cfg(feature = "async")]
impl<A, R> AsyncThread<A, R> {
    #[inline]
    pub(crate) fn set_recyclable(&mut self, recyclable: bool) {
        self.recycle = recyclable;
    }
}

#[cfg(feature = "async")]
#[cfg(any(feature = "lua54", feature = "luau"))]
impl<A, R> Drop for AsyncThread<A, R> {
    fn drop(&mut self) {
        if self.recycle {
            if let Some(lua) = self.thread.0.lua.try_lock() {
                unsafe {
                    // For Lua 5.4 this also closes all pending to-be-closed variables
                    if !lua.recycle_thread(&mut self.thread) {
                        #[cfg(feature = "lua54")]
                        if self.thread.status_inner(&lua) == ThreadStatus::Error {
                            #[cfg(not(feature = "vendored"))]
                            ffi::lua_resetthread(self.thread.state());
                            #[cfg(feature = "vendored")]
                            ffi::lua_closethread(self.thread.state(), lua.state());
                        }
                    }
                }
            }
        }
    }
}

#[cfg(feature = "async")]
impl<A: IntoLuaMulti, R: FromLuaMulti> Stream for AsyncThread<A, R> {
    type Item = Result<R>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let lua = self.thread.0.lua.lock();
        if self.thread.status_inner(&lua) != ThreadStatus::Resumable {
            return Poll::Ready(None);
        }

        let state = lua.state();
        let thread_state = self.thread.state();
        unsafe {
            let _sg = StackGuard::new(state);
            let _thread_sg = StackGuard::with_top(thread_state, 0);
            let _wg = WakerGuard::new(&lua, cx.waker());

            // This is safe as we are not moving the whole struct
            let this = self.get_unchecked_mut();
            let nresults = if let Some(args) = this.init_args.take() {
                this.thread.resume_inner(&lua, args)?
            } else {
                this.thread.resume_inner(&lua, ())?
            };

            if nresults == 1 && is_poll_pending(thread_state) {
                return Poll::Pending;
            }

            check_stack(state, nresults + 1)?;
            ffi::lua_xmove(thread_state, state, nresults);

            cx.waker().wake_by_ref();
            Poll::Ready(Some(R::from_stack_multi(nresults, &lua)))
        }
    }
}

#[cfg(feature = "async")]
impl<A: IntoLuaMulti, R: FromLuaMulti> Future for AsyncThread<A, R> {
    type Output = Result<R>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let lua = self.thread.0.lua.lock();
        if self.thread.status_inner(&lua) != ThreadStatus::Resumable {
            return Poll::Ready(Err(Error::CoroutineUnresumable));
        }

        let state = lua.state();
        let thread_state = self.thread.state();
        unsafe {
            let _sg = StackGuard::new(state);
            let _thread_sg = StackGuard::with_top(thread_state, 0);
            let _wg = WakerGuard::new(&lua, cx.waker());

            // This is safe as we are not moving the whole struct
            let this = self.get_unchecked_mut();
            let nresults = if let Some(args) = this.init_args.take() {
                this.thread.resume_inner(&lua, args)?
            } else {
                this.thread.resume_inner(&lua, ())?
            };

            if nresults == 1 && is_poll_pending(thread_state) {
                return Poll::Pending;
            }

            if ffi::lua_status(thread_state) == ffi::LUA_YIELD {
                // Ignore value returned via yield()
                cx.waker().wake_by_ref();
                return Poll::Pending;
            }

            check_stack(state, nresults + 1)?;
            ffi::lua_xmove(thread_state, state, nresults);

            Poll::Ready(R::from_stack_multi(nresults, &lua))
        }
    }
}

#[cfg(feature = "async")]
#[inline(always)]
unsafe fn is_poll_pending(state: *mut ffi::lua_State) -> bool {
    ffi::lua_tolightuserdata(state, -1) == Lua::poll_pending().0
}

#[cfg(feature = "async")]
struct WakerGuard<'lua, 'a> {
    lua: &'lua RawLua,
    prev: NonNull<Waker>,
    _phantom: PhantomData<&'a ()>,
}

#[cfg(feature = "async")]
impl<'lua, 'a> WakerGuard<'lua, 'a> {
    #[inline]
    pub fn new(lua: &'lua RawLua, waker: &'a Waker) -> Result<WakerGuard<'lua, 'a>> {
        let prev = unsafe { lua.set_waker(NonNull::from(waker)) };
        Ok(WakerGuard {
            lua,
            prev,
            _phantom: PhantomData,
        })
    }
}

#[cfg(feature = "async")]
impl Drop for WakerGuard<'_, '_> {
    fn drop(&mut self) {
        unsafe { self.lua.set_waker(self.prev) };
    }
}

#[cfg(test)]
mod assertions {
    use super::*;

    #[cfg(not(feature = "send"))]
    static_assertions::assert_not_impl_any!(Thread: Send);
    #[cfg(feature = "send")]
    static_assertions::assert_impl_all!(Thread: Send, Sync);
    #[cfg(all(feature = "async", not(feature = "send")))]
    static_assertions::assert_not_impl_any!(AsyncThread<(), ()>: Send);
    #[cfg(all(feature = "async", feature = "send"))]
    static_assertions::assert_impl_all!(AsyncThread<(), ()>: Send, Sync);
}
