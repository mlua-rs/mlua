use std::alloc::{self, Layout};
use std::os::raw::c_void;
use std::ptr;

pub(crate) static ALLOCATOR: ffi::lua_Alloc = allocator;

#[repr(C)]
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
    #[cfg(feature = "luau")]
    #[inline]
    pub(crate) unsafe fn get(state: *mut ffi::lua_State) -> *mut Self {
        let mut mem_state = ptr::null_mut();
        ffi::lua_getallocf(state, &mut mem_state);
        mlua_assert!(!mem_state.is_null(), "Luau state has no allocator userdata");
        mem_state as *mut MemoryState
    }

    #[cfg(not(feature = "luau"))]
    #[rustversion::since(1.85)]
    #[inline]
    #[allow(clippy::incompatible_msrv)]
    pub(crate) unsafe fn get(state: *mut ffi::lua_State) -> *mut Self {
        let mut mem_state = ptr::null_mut();
        if !ptr::fn_addr_eq(ffi::lua_getallocf(state, &mut mem_state), ALLOCATOR) {
            mem_state = ptr::null_mut();
        }
        mem_state as *mut MemoryState
    }

    #[cfg(not(feature = "luau"))]
    #[rustversion::before(1.85)]
    #[inline]
    pub(crate) unsafe fn get(state: *mut ffi::lua_State) -> *mut Self {
        let mut mem_state = ptr::null_mut();
        if ffi::lua_getallocf(state, &mut mem_state) != ALLOCATOR {
            mem_state = ptr::null_mut();
        }
        mem_state as *mut MemoryState
    }

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

    // This function is used primarily for calling `lua_pushcfunction` in lua5.1/jit/luau
    // to bypass the memory limit (if set).
    #[cfg(any(feature = "lua51", feature = "luajit", feature = "luau"))]
    #[inline]
    pub(crate) unsafe fn relax_limit_with(state: *mut ffi::lua_State, f: impl FnOnce()) {
        let mem_state = Self::get(state);
        if !mem_state.is_null() {
            (*mem_state).ignore_limit = true;
            f();
            (*mem_state).ignore_limit = false;
        } else {
            f();
        }
    }

    // Does nothing apart from calling `f()`, we don't need to bypass any limits
    #[cfg(any(feature = "lua52", feature = "lua53", feature = "lua54"))]
    #[inline]
    pub(crate) unsafe fn relax_limit_with(_state: *mut ffi::lua_State, f: impl FnOnce()) {
        f();
    }

    // Returns `true` if the memory limit was reached on the last memory operation
    #[cfg(feature = "luau")]
    #[inline]
    pub(crate) unsafe fn limit_reached(state: *mut ffi::lua_State) -> bool {
        (*Self::get(state)).limit_reached
    }
}

unsafe extern "C" fn allocator(
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
