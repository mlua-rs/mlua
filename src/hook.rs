use std::ffi::CStr;
use std::marker::PhantomData;
use std::os::raw::{c_char, c_int};

use crate::ffi::{self, lua_Debug, lua_State};
use crate::lua::Lua;
use crate::util::callback_error;

/// Contains information about currently executing Lua code.
///
/// The `Debug` structure is provided as a parameter to the hook function set with
/// [`Lua::set_hook`].  You may call the methods on this structure to retrieve information about the
/// Lua code executing at the time that the hook function was called.  Further information can be
/// found in the [Lua 5.3 documentaton][lua_doc].
///
/// [lua_doc]: https://www.lua.org/manual/5.3/manual.html#lua_Debug
/// [`Lua::set_hook`]: struct.Lua.html#method.set_hook
#[derive(Clone)]
pub struct Debug<'a> {
    ar: *mut lua_Debug,
    state: *mut lua_State,
    _phantom: PhantomData<&'a ()>,
}

impl<'a> Debug<'a> {
    /// Corresponds to the `n` what mask.
    pub fn names(&self) -> DebugNames<'a> {
        unsafe {
            mlua_assert!(
                ffi::lua_getinfo(self.state, cstr!("n"), self.ar) != 0,
                "lua_getinfo failed with `n`"
            );
            DebugNames {
                name: ptr_to_str((*self.ar).name),
                name_what: ptr_to_str((*self.ar).namewhat),
            }
        }
    }

    /// Corresponds to the `S` what mask.
    pub fn source(&self) -> DebugSource<'a> {
        unsafe {
            mlua_assert!(
                ffi::lua_getinfo(self.state, cstr!("S"), self.ar) != 0,
                "lua_getinfo failed with `S`"
            );
            DebugSource {
                source: ptr_to_str((*self.ar).source),
                short_src: ptr_to_str((*self.ar).short_src.as_ptr()),
                line_defined: (*self.ar).linedefined as i32,
                last_line_defined: (*self.ar).lastlinedefined as i32,
                what: ptr_to_str((*self.ar).what),
            }
        }
    }

    /// Corresponds to the `l` what mask. Returns the current line.
    pub fn curr_line(&self) -> i32 {
        unsafe {
            mlua_assert!(
                ffi::lua_getinfo(self.state, cstr!("l"), self.ar) != 0,
                "lua_getinfo failed with `l`"
            );
            (*self.ar).currentline as i32
        }
    }

    /// Corresponds to the `t` what mask. Returns true if the hook is in a function tail call, false
    /// otherwise.
    pub fn is_tail_call(&self) -> bool {
        unsafe {
            mlua_assert!(
                ffi::lua_getinfo(self.state, cstr!("t"), self.ar) != 0,
                "lua_getinfo failed with `t`"
            );
            (*self.ar).currentline != 0
        }
    }

    /// Corresponds to the `u` what mask.
    pub fn stack(&self) -> DebugStack {
        unsafe {
            mlua_assert!(
                ffi::lua_getinfo(self.state, cstr!("u"), self.ar) != 0,
                "lua_getinfo failed with `u`"
            );
            DebugStack {
                num_ups: (*self.ar).nups as i32,
                #[cfg(any(feature = "lua54", feature = "lua53", feature = "lua52"))]
                num_params: (*self.ar).nparams as i32,
                #[cfg(any(feature = "lua54", feature = "lua53", feature = "lua52"))]
                is_vararg: (*self.ar).isvararg != 0,
            }
        }
    }
}

#[derive(Clone, Debug)]
pub struct DebugNames<'a> {
    pub name: Option<&'a [u8]>,
    pub name_what: Option<&'a [u8]>,
}

#[derive(Clone, Debug)]
pub struct DebugSource<'a> {
    pub source: Option<&'a [u8]>,
    pub short_src: Option<&'a [u8]>,
    pub line_defined: i32,
    pub last_line_defined: i32,
    pub what: Option<&'a [u8]>,
}

#[derive(Copy, Clone, Debug)]
pub struct DebugStack {
    pub num_ups: i32,
    /// Requires `feature = "lua54/lua53/lua52"`
    #[cfg(any(feature = "lua54", feature = "lua53", feature = "lua52", doc))]
    pub num_params: i32,
    /// Requires `feature = "lua54/lua53/lua52"`
    #[cfg(any(feature = "lua54", feature = "lua53", feature = "lua52", doc))]
    pub is_vararg: bool,
}

/// Determines when a hook function will be called by Lua.
#[derive(Clone, Copy, Debug, Default)]
pub struct HookTriggers {
    /// Before a function call.
    pub on_calls: bool,
    /// When Lua returns from a function.
    pub on_returns: bool,
    /// Before executing a new line, or returning from a function call.
    pub every_line: bool,
    /// After a certain number of VM instructions have been executed.  When set to `Some(count)`,
    /// `count` is the number of VM instructions to execute before calling the hook.
    ///
    /// # Performance
    ///
    /// Setting this option to a low value can incur a very high overhead.
    pub every_nth_instruction: Option<u32>,
}

impl HookTriggers {
    // Compute the mask to pass to `lua_sethook`.
    pub(crate) fn mask(&self) -> c_int {
        let mut mask: c_int = 0;
        if self.on_calls {
            mask |= ffi::LUA_MASKCALL
        }
        if self.on_returns {
            mask |= ffi::LUA_MASKRET
        }
        if self.every_line {
            mask |= ffi::LUA_MASKLINE
        }
        if self.every_nth_instruction.is_some() {
            mask |= ffi::LUA_MASKCOUNT
        }
        mask
    }

    // Returns the `count` parameter to pass to `lua_sethook`, if applicable. Otherwise, zero is
    // returned.
    pub(crate) fn count(&self) -> c_int {
        self.every_nth_instruction.unwrap_or(0) as c_int
    }
}

pub(crate) unsafe extern "C" fn hook_proc(state: *mut lua_State, ar: *mut lua_Debug) {
    callback_error(state, |_| {
        let debug = Debug {
            ar,
            state,
            _phantom: PhantomData,
        };

        let lua = Lua::make_from_ptr(state);
        let hook_cb = mlua_expect!(lua.hook_callback(), "no hook callback set in hook_proc");

        #[allow(clippy::match_wild_err_arm)]
        match hook_cb.try_borrow_mut() {
            Ok(mut b) => (&mut *b)(&lua, debug),
            Err(_) => mlua_panic!("Lua should not allow hooks to be called within another hook"),
        }?;

        Ok(())
    });
}

unsafe fn ptr_to_str<'a>(input: *const c_char) -> Option<&'a [u8]> {
    if input.is_null() {
        None
    } else {
        Some(CStr::from_ptr(input).to_bytes())
    }
}
