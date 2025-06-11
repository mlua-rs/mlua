//! Contains definitions from `Require.h`.

use std::os::raw::{c_char, c_int, c_void};

use super::lua::lua_State;

pub const LUA_REGISTERED_MODULES_TABLE: *const c_char = cstr!("_REGISTEREDMODULES");

#[repr(C)]
pub enum luarequire_NavigateResult {
    Success,
    Ambiguous,
    NotFound,
}

// Functions returning WriteSuccess are expected to set their size_out argument
// to the number of bytes written to the buffer. If WriteBufferTooSmall is
// returned, size_out should be set to the required buffer size.
#[repr(C)]
pub enum luarequire_WriteResult {
    Success,
    BufferTooSmall,
    Failure,
}

#[repr(C)]
pub struct luarequire_Configuration {
    // Returns whether requires are permitted from the given chunkname.
    pub is_require_allowed: unsafe extern "C-unwind" fn(
        L: *mut lua_State,
        ctx: *mut c_void,
        requirer_chunkname: *const c_char,
    ) -> bool,

    // Resets the internal state to point at the requirer module.
    pub reset: unsafe extern "C-unwind" fn(
        L: *mut lua_State,
        ctx: *mut c_void,
        requirer_chunkname: *const c_char,
    ) -> luarequire_NavigateResult,

    // Resets the internal state to point at an aliased module, given its exact path from a configuration
    // file. This function is only called when an alias's path cannot be resolved relative to its
    // configuration file.
    pub jump_to_alias: unsafe extern "C-unwind" fn(
        L: *mut lua_State,
        ctx: *mut c_void,
        path: *const c_char,
    ) -> luarequire_NavigateResult,

    // Navigates through the context by making mutations to the internal state.
    pub to_parent:
        unsafe extern "C-unwind" fn(L: *mut lua_State, ctx: *mut c_void) -> luarequire_NavigateResult,
    pub to_child: unsafe extern "C-unwind" fn(
        L: *mut lua_State,
        ctx: *mut c_void,
        name: *const c_char,
    ) -> luarequire_NavigateResult,

    // Returns whether the context is currently pointing at a module.
    pub is_module_present: unsafe extern "C-unwind" fn(L: *mut lua_State, ctx: *mut c_void) -> bool,

    // Provides a chunkname for the current module. This will be accessible through the debug library. This
    // function is only called if is_module_present returns true.
    pub get_chunkname: unsafe extern "C-unwind" fn(
        L: *mut lua_State,
        ctx: *mut c_void,
        buffer: *mut c_char,
        buffer_size: usize,
        size_out: *mut usize,
    ) -> luarequire_WriteResult,

    // Provides a loadname that identifies the current module and is passed to load. This function
    // is only called if is_module_present returns true.
    pub get_loadname: unsafe extern "C-unwind" fn(
        L: *mut lua_State,
        ctx: *mut c_void,
        buffer: *mut c_char,
        buffer_size: usize,
        size_out: *mut usize,
    ) -> luarequire_WriteResult,

    // Provides a cache key representing the current module. This function is only called if
    // is_module_present returns true.
    pub get_cache_key: unsafe extern "C-unwind" fn(
        L: *mut lua_State,
        ctx: *mut c_void,
        buffer: *mut c_char,
        buffer_size: usize,
        size_out: *mut usize,
    ) -> luarequire_WriteResult,

    // Returns whether a configuration file is present in the current context.
    // If not, require-by-string will call to_parent until either a configuration file is present or
    // NAVIGATE_FAILURE is returned (at root).
    pub is_config_present: unsafe extern "C-unwind" fn(L: *mut lua_State, ctx: *mut c_void) -> bool,

    // Parses the configuration file in the current context for the given alias and returns its
    // value or WRITE_FAILURE if not found. This function is only called if is_config_present
    // returns true. If this function pointer is set, get_config must not be set. Opting in to this
    // function pointer disables parsing configuration files internally and can be used for finer
    // control over the configuration file parsing process.
    pub get_alias: Option<
        unsafe extern "C-unwind" fn(
            L: *mut lua_State,
            ctx: *mut c_void,
            alias: *const c_char,
            buffer: *mut c_char,
            buffer_size: usize,
            size_out: *mut usize,
        ) -> luarequire_WriteResult,
    >,

    // Provides the contents of the configuration file in the current context. This function is only called
    // if is_config_present returns true. If this function pointer is set, get_alias must not be set. Opting
    // in to this function pointer enables parsing configuration files internally.
    pub get_config: Option<
        unsafe extern "C-unwind" fn(
            L: *mut lua_State,
            ctx: *mut c_void,
            buffer: *mut c_char,
            buffer_size: usize,
            size_out: *mut usize,
        ) -> luarequire_WriteResult,
    >,

    // Executes the module and places the result on the stack. Returns the number of results placed on the
    // stack.
    // Returning -1 directs the requiring thread to yield. In this case, this thread should be resumed with
    // the module result pushed onto its stack.
    pub load: unsafe extern "C-unwind" fn(
        L: *mut lua_State,
        ctx: *mut c_void,
        path: *const c_char,
        chunkname: *const c_char,
        loadname: *const c_char,
    ) -> c_int,
}

// Populates function pointers in the given luarequire_Configuration.
pub type luarequire_Configuration_init = unsafe extern "C-unwind" fn(config: *mut luarequire_Configuration);

unsafe extern "C-unwind" {
    // Initializes and pushes the require closure onto the stack without registration.
    pub fn luarequire_pushrequire(
        L: *mut lua_State,
        config_init: luarequire_Configuration_init,
        ctx: *mut c_void,
    ) -> c_int;

    // Initializes the require library and registers it globally.
    pub fn luaopen_require(L: *mut lua_State, config_init: luarequire_Configuration_init, ctx: *mut c_void);

    // Initializes and pushes a "proxyrequire" closure onto the stack.
    //
    // The closure takes two parameters: the string path to resolve and the chunkname of an existing
    // module.
    pub fn luarequire_pushproxyrequire(
        L: *mut lua_State,
        config_init: luarequire_Configuration_init,
        ctx: *mut c_void,
    ) -> c_int;

    // Registers an aliased require path to a result.
    //
    // After registration, the given result will always be immediately returned when the given path is
    // required.
    // Expects the path and table to be passed as arguments on the stack.
    pub fn luarequire_registermodule(L: *mut lua_State) -> c_int;

    // Clears the entry associated with the given cache key from the require cache.
    // Expects the cache key to be passed as an argument on the stack.
    pub fn luarequire_clearcacheentry(L: *mut lua_State) -> c_int;

    // Clears all entries from the require cache.
    pub fn luarequire_clearcache(L: *mut lua_State) -> c_int;
}
