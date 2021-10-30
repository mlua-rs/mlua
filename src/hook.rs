use std::ffi::CStr;
use std::marker::PhantomData;
use std::ops::{BitOr, BitOrAssign};
use std::os::raw::{c_char, c_int};

use crate::ffi::{self, lua_Debug, lua_State};
use crate::lua::Lua;
use crate::util::callback_error;

/// Contains information about currently executing Lua code.
///
/// The `Debug` structure is provided as a parameter to the hook function set with
/// [`Lua::set_hook`]. You may call the methods on this structure to retrieve information about the
/// Lua code executing at the time that the hook function was called. Further information can be
/// found in the [Lua 5.3 documentation][lua_doc].
///
/// The `Debug` structure can also be accessed arbitrarily with [`Lua::with_debug_iter`].
///
/// [lua_doc]: https://www.lua.org/manual/5.3/manual.html#lua_Debug
/// [`Lua::set_hook`]: crate::Lua::set_hook
/// [`Lua::with_debug_iter`]: crate::Lua::with_debug_iter
#[derive(Clone)]
pub struct Debug<'a> {
    ar: *mut lua_Debug,
    state: *mut lua_State,
    _phantom: PhantomData<&'a ()>,
}

impl<'a> Debug<'a> {
    pub(crate) unsafe fn new(ar: &'a mut lua_Debug, state: *mut lua_State) -> Self {
        Debug {
            ar,
            state,
            _phantom: PhantomData,
        }
    }

    /// Returns the specific event that triggered the hook.
    ///
    /// For [Lua 5.1] `DebugEvent::TailCall` is used for return events to indicate a return
    /// from a function that did a tail call.
    ///
    /// [Lua 5.1]: https://www.lua.org/manual/5.1/manual.html#pdf-LUA_HOOKTAILRET
    pub fn event(&self) -> DebugEvent {
        unsafe {
            match (*self.ar).event {
                ffi::LUA_HOOKCALL => DebugEvent::Call,
                ffi::LUA_HOOKRET => DebugEvent::Ret,
                ffi::LUA_HOOKTAILCALL => DebugEvent::TailCall,
                ffi::LUA_HOOKLINE => DebugEvent::Line,
                ffi::LUA_HOOKCOUNT => DebugEvent::Count,
                event => mlua_panic!("Unknown Lua event code: {}", event),
            }
        }
    }

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

/// Represents a specific event that triggered the hook.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DebugEvent {
    Call,
    Ret,
    TailCall,
    Line,
    Count,
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
    /// After a certain number of VM instructions have been executed. When set to `Some(count)`,
    /// `count` is the number of VM instructions to execute before calling the hook.
    ///
    /// # Performance
    ///
    /// Setting this option to a low value can incur a very high overhead.
    pub every_nth_instruction: Option<u32>,
}

impl HookTriggers {
    /// Returns a new instance of `HookTriggers` with [`on_calls`] trigger set.
    ///
    /// [`on_calls`]: #structfield.on_calls
    pub fn on_calls() -> Self {
        HookTriggers {
            on_calls: true,
            ..Default::default()
        }
    }

    /// Returns a new instance of `HookTriggers` with [`on_returns`] trigger set.
    ///
    /// [`on_returns`]: #structfield.on_returns
    pub fn on_returns() -> Self {
        HookTriggers {
            on_returns: true,
            ..Default::default()
        }
    }

    /// Returns a new instance of `HookTriggers` with [`every_line`] trigger set.
    ///
    /// [`every_line`]: #structfield.every_line
    pub fn every_line() -> Self {
        HookTriggers {
            every_line: true,
            ..Default::default()
        }
    }

    /// Returns a new instance of `HookTriggers` with [`every_nth_instruction`] trigger set.
    ///
    /// [`every_nth_instruction`]: #structfield.every_nth_instruction
    pub fn every_nth_instruction(n: u32) -> Self {
        HookTriggers {
            every_nth_instruction: Some(n),
            ..Default::default()
        }
    }

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

impl BitOr for HookTriggers {
    type Output = Self;

    fn bitor(mut self, rhs: Self) -> Self::Output {
        self.on_calls |= rhs.on_calls;
        self.on_returns |= rhs.on_returns;
        self.every_line |= rhs.every_line;
        if self.every_nth_instruction.is_none() && rhs.every_nth_instruction.is_some() {
            self.every_nth_instruction = rhs.every_nth_instruction;
        }
        self
    }
}

impl BitOrAssign for HookTriggers {
    fn bitor_assign(&mut self, rhs: Self) {
        *self = *self | rhs;
    }
}

pub(crate) unsafe extern "C" fn hook_proc(state: *mut lua_State, ar: *mut lua_Debug) {
    callback_error(state, |_| {
        let debug = Debug {
            ar,
            state,
            _phantom: PhantomData,
        };

        let lua = mlua_expect!(Lua::make_from_ptr(state), "cannot make Lua instance");
        let hook_cb = mlua_expect!(lua.hook_callback(), "no hook callback set in hook_proc");

        #[allow(clippy::match_wild_err_arm)]
        match hook_cb.try_borrow_mut() {
            Ok(mut b) => (&mut *b)(&lua, debug),
            Err(_) => mlua_panic!("Lua should not allow hooks to be called within another hook"),
        }?;

        Ok(())
    })
}

unsafe fn ptr_to_str<'a>(input: *const c_char) -> Option<&'a [u8]> {
    if input.is_null() {
        None
    } else {
        Some(CStr::from_ptr(input).to_bytes())
    }
}
