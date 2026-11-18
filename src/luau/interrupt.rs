use std::ptr;
use std::sync::Arc;

use parking_lot::Mutex;

use crate::Lua;

/// A handle for arming an interrupt registered with [`Lua::set_interrupt_oneshot`].
///
/// Removing or replacing the interrupt, or destroying the VM, invalidates all clones.
/// Dropping a handle does not cancel pending requests.
#[derive(Clone, Debug)]
pub struct InterruptHandle {
    inner: Arc<Mutex<InterruptState>>,
}

#[derive(Debug)]
struct InterruptState {
    #[cfg(feature = "luau")]
    slot: *mut Option<ffi::lua_InterruptFunction>,
    pending: bool,
    running: bool,
}

// SAFETY: Only the interrupt slot is accessed through this pointer.
unsafe impl Send for InterruptState {}

impl InterruptHandle {
    /// Requests one callback invocation at a Luau execution safepoint.
    ///
    /// Pending requests coalesce and are consumed before invocation, regardless of the callback's
    /// outcome. Requests made during the callback are deferred until it exits.
    ///
    /// Returns `true` if accepted, including when already armed, or `false` if the handle is
    /// invalidated. Later invalidation can cancel an accepted request.
    pub fn arm(&self) -> bool {
        let mut state = self.inner.lock();
        if state.slot.is_null() {
            return false;
        }
        state.pending = true;
        if !state.running {
            unsafe { state.slot.write(Some(Lua::interrupt_proc)) };
        }
        true
    }

    /// Cancels the pending request, including one queued during a callback.
    ///
    /// A callback already claimed for delivery may still run. Later calls to [`Self::arm`]
    /// can schedule another request.
    pub fn disarm(&self) {
        let mut state = self.inner.lock();
        if state.slot.is_null() {
            return;
        }
        state.pending = false;
        if !state.running {
            unsafe { state.slot.write(None) };
        }
    }

    // The caller must invalidate this registration before freeing the callback slot, and must
    // invalidate any previous registration before constructing this one.
    pub(crate) unsafe fn new(state: *mut ffi::lua_State) -> Self {
        let slot = ptr::addr_of_mut!((*ffi::lua_callbacks(state)).interrupt);
        slot.write(None);
        Self {
            inner: Arc::new(Mutex::new(InterruptState {
                slot,
                pending: false,
                running: false,
            })),
        }
    }

    pub(crate) fn invalidate(&self) {
        let mut state = self.inner.lock();
        if !state.slot.is_null() {
            unsafe { state.slot.write(None) };
            state.slot = ptr::null_mut();
        }
        state.pending = false;
    }

    pub(crate) fn begin(&self) -> Option<InterruptGuard> {
        let mut state = self.inner.lock();
        if state.slot.is_null() || !state.pending || state.running {
            return None;
        }
        state.pending = false;
        state.running = true;
        unsafe { state.slot.write(None) };
        Some(InterruptGuard(self.clone()))
    }
}

#[must_use = "keep the guard alive until the interrupt callback finishes"]
pub(crate) struct InterruptGuard(InterruptHandle);

impl Drop for InterruptGuard {
    fn drop(&mut self) {
        let mut state = self.0.inner.lock();
        state.running = false;
        // A callback may have replaced or removed its own registration.
        if state.pending && !state.slot.is_null() {
            unsafe { state.slot.write(Some(Lua::interrupt_proc)) };
        }
    }
}
