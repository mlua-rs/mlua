use std::os::raw::{c_int, c_void};
use std::sync::atomic::{AtomicPtr, Ordering};
use std::sync::Arc;

use crate::error::{Error, Result};
#[allow(unused)]
use crate::lua::Lua;
use crate::types::LuaRef;
use crate::util::{check_stack, error_traceback_thread, pop_error, StackGuard};
use crate::value::{FromLuaMulti, IntoLuaMulti};

#[cfg(not(feature = "luau"))]
use crate::{
    hook::{Debug, HookTriggers},
    types::MaybeSend,
};

#[cfg(feature = "async")]
use {
    crate::value::MultiValue,
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
    /// The thread was just created, or is suspended because it has called `coroutine.yield`.
    ///
    /// If a thread is in this state, it can be resumed by calling [`Thread::resume`].
    ///
    /// [`Thread::resume`]: crate::Thread::resume
    Resumable,
    /// Either the thread has finished executing, or the thread is currently running.
    Unresumable,
    /// The thread has raised a Lua error during execution.
    Error,
}

/// Handle to an internal Lua thread (coroutine).
#[derive(Clone, Debug)]
pub struct Thread<'lua>(
    pub(crate) LuaRef<'lua>,
    pub(crate) Arc<AtomicPtr<ffi::lua_State>>,
);

/// Owned handle to an internal Lua thread (coroutine).
///
/// The owned handle holds a *strong* reference to the current Lua instance.
/// Be warned, if you place it into a Lua type (eg. [`UserData`] or a Rust callback), it is *very easy*
/// to accidentally cause reference cycles that would prevent destroying Lua instance.
///
/// [`UserData`]: crate::UserData
#[cfg(feature = "unstable")]
#[cfg_attr(docsrs, doc(cfg(feature = "unstable")))]
#[derive(Clone, Debug)]
pub struct OwnedThread(
    pub(crate) crate::types::LuaOwnedRef,
    pub(crate) *mut ffi::lua_State,
);

#[cfg(feature = "unstable")]
impl OwnedThread {
    /// Get borrowed handle to the underlying Lua table.
    #[cfg_attr(feature = "send", allow(unused))]
    pub const fn to_ref(&self) -> Thread {
        Thread(self.0.to_ref(), self.1)
    }
}

/// Thread (coroutine) representation as an async [`Future`] or [`Stream`].
///
/// Requires `feature = "async"`
///
/// [`Future`]: std::future::Future
/// [`Stream`]: futures_util::stream::Stream
#[cfg(feature = "async")]
#[cfg_attr(docsrs, doc(cfg(feature = "async")))]
#[must_use = "futures do nothing unless you `.await` or poll them"]
pub struct AsyncThread<'lua, R> {
    thread: Thread<'lua>,
    init_args: Option<Result<MultiValue<'lua>>>,
    ret: PhantomData<R>,
    recycle: bool,
}

impl<'lua> Thread<'lua> {
    #[inline(always)]
    pub(crate) fn new(r#ref: LuaRef<'lua>) -> Self {
        let state = unsafe { ffi::lua_tothread(r#ref.lua.ref_thread(), r#ref.index) };
        Thread(r#ref, Arc::new(AtomicPtr::new(state)))
    }

    #[inline(always)]
    fn state(&self) -> *mut ffi::lua_State {
        self.1.load(Ordering::Relaxed)
    }

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
        A: IntoLuaMulti<'lua>,
        R: FromLuaMulti<'lua>,
    {
        let lua = self.0.lua;
        let state = lua.state();
        let thread_state = self.state();
        unsafe {
            let _sg = StackGuard::new(state);
            let _thread_sg = StackGuard::with_top(thread_state, 0);

            let nresults = self.resume_inner(args)?;
            check_stack(state, nresults + 1)?;
            ffi::lua_xmove(thread_state, state, nresults);

            R::from_stack_multi(nresults, lua)
        }
    }

    /// Resumes execution of this thread.
    ///
    /// It's similar to `resume()` but leaves `nresults` values on the thread stack.
    unsafe fn resume_inner<A: IntoLuaMulti<'lua>>(&self, args: A) -> Result<c_int> {
        let lua = self.0.lua;
        let state = lua.state();
        let thread_state = self.state();

        if self.status() != ThreadStatus::Resumable {
            return Err(Error::CoroutineInactive);
        }

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
            protect_lua!(state, 0, 1, |state| error_traceback_thread(
                state,
                thread_state
            ))?;
            return Err(pop_error(state, ret));
        }

        Ok(nresults)
    }

    /// Gets the status of the thread.
    pub fn status(&self) -> ThreadStatus {
        let thread_state = self.state();
        unsafe {
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

    /// Sets a 'hook' function that will periodically be called as Lua code executes.
    ///
    /// This function is similar or [`Lua::set_hook()`] except that it sets for the thread.
    /// To remove a hook call [`Lua::remove_hook()`].
    #[cfg(not(feature = "luau"))]
    #[cfg_attr(docsrs, doc(cfg(not(feature = "luau"))))]
    pub fn set_hook<F>(&self, triggers: HookTriggers, callback: F)
    where
        F: Fn(&Lua, Debug) -> Result<()> + MaybeSend + 'static,
    {
        let lua = self.0.lua;
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
    pub fn reset(&self, func: crate::function::Function<'lua>) -> Result<()> {
        let lua = self.0.lua;
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

    /// Converts Thread to an AsyncThread which implements [`Future`] and [`Stream`] traits.
    ///
    /// `args` are passed as arguments to the thread function for first call.
    /// The object calls [`resume()`] while polling and also allows to run rust futures
    /// to completion using an executor.
    ///
    /// Using AsyncThread as a Stream allows to iterate through `coroutine.yield()`
    /// values whereas Future version discards that values and poll until the final
    /// one (returned from the thread function).
    ///
    /// Requires `feature = "async"`
    ///
    /// [`Future`]: std::future::Future
    /// [`Stream`]: futures_util::stream::Stream
    /// [`resume()`]: https://www.lua.org/manual/5.4/manual.html#lua_resume
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
        A: IntoLuaMulti<'lua>,
        R: FromLuaMulti<'lua>,
    {
        let args = args.into_lua_multi(self.0.lua);
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
    /// This mode ideally should be used together with the global sandbox mode [`Lua::sandbox()`].
    ///
    /// Please note that Luau links environment table with chunk when loading it into Lua state.
    /// Therefore you need to load chunks into a thread to link with the thread environment.
    ///
    /// # Examples
    ///
    /// ```
    /// # use mlua::{Lua, Result};
    /// # fn main() -> Result<()> {
    /// let lua = Lua::new();
    /// let thread = lua.create_thread(lua.create_function(|lua2, ()| {
    ///     lua2.load("var = 123").exec()?;
    ///     assert_eq!(lua2.globals().get::<_, u32>("var")?, 123);
    ///     Ok(())
    /// })?)?;
    /// thread.sandbox()?;
    /// thread.resume(())?;
    ///
    /// // The global environment should be unchanged
    /// assert_eq!(lua.globals().get::<_, Option<u32>>("var")?, None);
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// Requires `feature = "luau"`
    #[cfg(any(feature = "luau", docsrs))]
    #[cfg_attr(docsrs, doc(cfg(feature = "luau")))]
    #[doc(hidden)]
    pub fn sandbox(&self) -> Result<()> {
        let lua = self.0.lua;
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

    /// Convert this handle to owned version.
    #[cfg(all(feature = "unstable", any(not(feature = "send"), doc)))]
    #[cfg_attr(docsrs, doc(cfg(all(feature = "unstable", not(feature = "send")))))]
    #[inline]
    pub fn into_owned(self) -> OwnedThread {
        OwnedThread(self.0.into_owned(), self.1)
    }
}

impl<'lua> PartialEq for Thread<'lua> {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

// Additional shortcuts
#[cfg(feature = "unstable")]
impl OwnedThread {
    /// Resumes execution of this thread.
    ///
    /// See [`Thread::resume()`] for more details.
    pub fn resume<'lua, A, R>(&'lua self, args: A) -> Result<R>
    where
        A: IntoLuaMulti<'lua>,
        R: FromLuaMulti<'lua>,
    {
        self.to_ref().resume(args)
    }

    /// Gets the status of the thread.
    pub fn status(&self) -> ThreadStatus {
        self.to_ref().status()
    }
}

#[cfg(feature = "async")]
impl<'lua, R> AsyncThread<'lua, R> {
    #[inline]
    pub(crate) fn set_recyclable(&mut self, recyclable: bool) {
        self.recycle = recyclable;
    }
}

#[cfg(feature = "async")]
#[cfg(any(feature = "lua54", feature = "luau"))]
impl<'lua, R> Drop for AsyncThread<'lua, R> {
    fn drop(&mut self) {
        if self.recycle {
            unsafe {
                let lua = self.thread.0.lua;
                // For Lua 5.4 this also closes all pending to-be-closed variables
                if !lua.recycle_thread(&mut self.thread) {
                    #[cfg(feature = "lua54")]
                    if self.thread.status() == ThreadStatus::Error {
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

#[cfg(feature = "async")]
impl<'lua, R> Stream for AsyncThread<'lua, R>
where
    R: FromLuaMulti<'lua>,
{
    type Item = Result<R>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        if self.thread.status() != ThreadStatus::Resumable {
            return Poll::Ready(None);
        }

        let lua = self.thread.0.lua;
        let state = lua.state();
        let thread_state = self.thread.state();
        unsafe {
            let _sg = StackGuard::new(state);
            let _thread_sg = StackGuard::with_top(thread_state, 0);
            let _wg = WakerGuard::new(lua, cx.waker());

            // This is safe as we are not moving the whole struct
            let this = self.get_unchecked_mut();
            let nresults = if let Some(args) = this.init_args.take() {
                this.thread.resume_inner(args?)?
            } else {
                this.thread.resume_inner(())?
            };

            if nresults == 1 && is_poll_pending(thread_state) {
                return Poll::Pending;
            }

            check_stack(state, nresults + 1)?;
            ffi::lua_xmove(thread_state, state, nresults);

            cx.waker().wake_by_ref();
            Poll::Ready(Some(R::from_stack_multi(nresults, lua)))
        }
    }
}

#[cfg(feature = "async")]
impl<'lua, R> Future for AsyncThread<'lua, R>
where
    R: FromLuaMulti<'lua>,
{
    type Output = Result<R>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        if self.thread.status() != ThreadStatus::Resumable {
            return Poll::Ready(Err(Error::CoroutineInactive));
        }

        let lua = self.thread.0.lua;
        let state = lua.state();
        let thread_state = self.thread.state();
        unsafe {
            let _sg = StackGuard::new(state);
            let _thread_sg = StackGuard::with_top(thread_state, 0);
            let _wg = WakerGuard::new(lua, cx.waker());

            // This is safe as we are not moving the whole struct
            let this = self.get_unchecked_mut();
            let nresults = if let Some(args) = this.init_args.take() {
                this.thread.resume_inner(args?)?
            } else {
                this.thread.resume_inner(())?
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

            Poll::Ready(R::from_stack_multi(nresults, lua))
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
    lua: &'lua Lua,
    prev: NonNull<Waker>,
    _phantom: PhantomData<&'a ()>,
}

#[cfg(feature = "async")]
impl<'lua, 'a> WakerGuard<'lua, 'a> {
    #[inline]
    pub fn new(lua: &'lua Lua, waker: &'a Waker) -> Result<WakerGuard<'lua, 'a>> {
        let prev = unsafe { lua.set_waker(NonNull::from(waker)) };
        Ok(WakerGuard {
            lua,
            prev,
            _phantom: PhantomData,
        })
    }
}

#[cfg(feature = "async")]
impl<'lua, 'a> Drop for WakerGuard<'lua, 'a> {
    fn drop(&mut self) {
        unsafe { self.lua.set_waker(self.prev) };
    }
}

#[cfg(test)]
mod assertions {
    use super::*;

    static_assertions::assert_not_impl_any!(Thread: Send);
}
