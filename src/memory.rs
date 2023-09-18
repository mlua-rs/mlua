use std::prelude::v1::*;

use std::alloc::{self, Layout};
use std::ffi::c_void;
use std::ptr;

#[cfg(feature = "luau")]
use crate::lua::ExtraData;

pub(crate) static ALLOCATOR: ffi::lua_Alloc = allocator;

#[derive(Default)]
pub(crate) struct MemoryState {
    used_memory: isize,
    memory_limit: isize,
    // Can be set to temporary ignore the memory limit.
    // This is used when calling `lua_pushcfunction` for lua5.1/jit/luau.
    ignore_limit: bool,
    // Indicates that the memory limit was reached on the last allocation.
    #[cfg(feature = "luau")]
    limit_reached: bool,
}

impl MemoryState {
    #[inline]
    pub(crate) fn used_memory(&self) -> usize {
        self.used_memory as usize
    }

    #[inline]
    pub(crate) fn memory_limit(&self) -> usize {
        self.memory_limit as usize
    }

    #[inline]
    pub(crate) fn set_memory_limit(&mut self, limit: usize) -> usize {
        let prev_limit = self.memory_limit;
        self.memory_limit = limit as isize;
        prev_limit as usize
    }

    // This function is used primarily for calling `lua_pushcfunction` in lua5.1/jit
    // to bypass the memory limit (if set).
    #[cfg(any(feature = "lua51", feature = "luajit"))]
    #[inline]
    pub(crate) unsafe fn relax_limit_with(state: *mut ffi::lua_State, f: impl FnOnce()) {
        let mut mem_state: *mut c_void = ptr::null_mut();
        if ffi::lua_getallocf(state, &mut mem_state) == ALLOCATOR {
            (*(mem_state as *mut MemoryState)).ignore_limit = true;
            f();
            (*(mem_state as *mut MemoryState)).ignore_limit = false;
        } else {
            f();
        }
    }

    // Same as the above but for Luau
    // It does not have `lua_getallocf` function, so instead we use `lua_callbacks`
    #[cfg(feature = "luau")]
    #[inline]
    pub(crate) unsafe fn relax_limit_with(state: *mut ffi::lua_State, f: impl FnOnce()) {
        let extra = (*ffi::lua_callbacks(state)).userdata as *mut ExtraData;
        if extra.is_null() {
            return f();
        }
        let mem_state = (*extra).mem_state();
        (*mem_state.as_ptr()).ignore_limit = true;
        f();
        (*mem_state.as_ptr()).ignore_limit = false;
    }

    // Does nothing apart from calling `f()`, we don't need to bypass any limits
    #[cfg(any(feature = "lua52", feature = "lua53", feature = "lua54"))]
    #[inline]
    pub(crate) unsafe fn relax_limit_with(_state: *mut ffi::lua_State, f: impl FnOnce()) {
        f();
    }

    // Returns `true` if the memory limit was reached on the last memory operation
    #[cfg(feature = "luau")]
    pub(crate) unsafe fn limit_reached(state: *mut ffi::lua_State) -> bool {
        let extra = (*ffi::lua_callbacks(state)).userdata as *mut ExtraData;
        if extra.is_null() {
            return false;
        }
        (*(*extra).mem_state().as_ptr()).limit_reached
    }
}

unsafe extern "C-unwind" fn allocator(
    extra: *mut c_void,
    ptr: *mut c_void,
    osize: usize,
    nsize: usize,
) -> *mut c_void {
    let mem_state = &mut *(extra as *mut MemoryState);
    #[cfg(feature = "luau")]
    {
        // Reset the flag
        mem_state.limit_reached = false;
    }

    if nsize == 0 {
        // Free memory
        if !ptr.is_null() {
            let layout = Layout::from_size_align_unchecked(osize, ffi::SYS_MIN_ALIGN);
            alloc::dealloc(ptr as *mut u8, layout);
            mem_state.used_memory -= osize as isize;
        }
        return ptr::null_mut();
    }

    // Do not allocate more than isize::MAX
    if nsize > isize::MAX as usize {
        return ptr::null_mut();
    }

    // Are we fit to the memory limits?
    let mut mem_diff = nsize as isize;
    if !ptr.is_null() {
        mem_diff -= osize as isize;
    }
    let mem_limit = mem_state.memory_limit;
    let new_used_memory = mem_state.used_memory + mem_diff;
    if mem_limit > 0 && new_used_memory > mem_limit && !mem_state.ignore_limit {
        #[cfg(feature = "luau")]
        {
            mem_state.limit_reached = true;
        }
        return ptr::null_mut();
    }
    mem_state.used_memory += mem_diff;

    if ptr.is_null() {
        // Allocate new memory
        let new_layout = match Layout::from_size_align(nsize, ffi::SYS_MIN_ALIGN) {
            Ok(layout) => layout,
            Err(_) => return ptr::null_mut(),
        };
        let new_ptr = alloc::alloc(new_layout) as *mut c_void;
        if new_ptr.is_null() {
            alloc::handle_alloc_error(new_layout);
        }
        return new_ptr;
    }

    // Reallocate memory
    let old_layout = Layout::from_size_align_unchecked(osize, ffi::SYS_MIN_ALIGN);
    let new_ptr = alloc::realloc(ptr as *mut u8, old_layout, nsize) as *mut c_void;
    if new_ptr.is_null() {
        alloc::handle_alloc_error(old_layout);
    }
    new_ptr
}
