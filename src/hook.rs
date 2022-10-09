use std::cell::UnsafeCell;
#[cfg(not(feature = "luau"))]
use std::ops::{BitOr, BitOrAssign};
use std::os::raw::c_int;

use crate::ffi::{self, lua_Debug};
use crate::lua::Lua;
use crate::util::ptr_to_cstr_bytes;

/// Contains information about currently executing Lua code.
///
/// The `Debug` structure is provided as a parameter to the hook function set with
/// [`Lua::set_hook`]. You may call the methods on this structure to retrieve information about the
/// Lua code executing at the time that the hook function was called. Further information can be
/// found in the Lua [documentation][lua_doc].
///
/// [lua_doc]: https://www.lua.org/manual/5.4/manual.html#lua_Debug
/// [`Lua::set_hook`]: crate::Lua::set_hook
pub struct Debug<'lua> {
    lua: &'lua Lua,
    ar: ActivationRecord,
    #[cfg(feature = "luau")]
    level: c_int,
}

impl<'lua> Debug<'lua> {
    #[cfg(not(feature = "luau"))]
    pub(crate) fn new(lua: &'lua Lua, ar: *mut lua_Debug) -> Self {
        Debug {
            lua,
            ar: ActivationRecord::Borrowed(ar),
        }
    }

    pub(crate) fn new_owned(lua: &'lua Lua, _level: c_int, ar: lua_Debug) -> Self {
        Debug {
            lua,
            ar: ActivationRecord::Owned(UnsafeCell::new(ar)),
            #[cfg(feature = "luau")]
            level: _level,
        }
    }

    /// Returns the specific event that triggered the hook.
    ///
    /// For [Lua 5.1] `DebugEvent::TailCall` is used for return events to indicate a return
    /// from a function that did a tail call.
    ///
    /// [Lua 5.1]: https://www.lua.org/manual/5.1/manual.html#pdf-LUA_HOOKTAILRET
    #[cfg(not(feature = "luau"))]
    #[cfg_attr(docsrs, doc(cfg(not(feature = "luau"))))]
    pub fn event(&self) -> DebugEvent {
        unsafe {
            match (*self.ar.get()).event {
                ffi::LUA_HOOKCALL => DebugEvent::Call,
                ffi::LUA_HOOKRET => DebugEvent::Ret,
                ffi::LUA_HOOKTAILCALL => DebugEvent::TailCall,
                ffi::LUA_HOOKLINE => DebugEvent::Line,
                ffi::LUA_HOOKCOUNT => DebugEvent::Count,
                event => DebugEvent::Unknown(event),
            }
        }
    }

    /// Corresponds to the `n` what mask.
    pub fn names(&self) -> DebugNames<'lua> {
        unsafe {
            #[cfg(not(feature = "luau"))]
            mlua_assert!(
                ffi::lua_getinfo(self.lua.state, cstr!("n"), self.ar.get()) != 0,
                "lua_getinfo failed with `n`"
            );
            #[cfg(feature = "luau")]
            mlua_assert!(
                ffi::lua_getinfo(self.lua.state, self.level, cstr!("n"), self.ar.get()) != 0,
                "lua_getinfo failed with `n`"
            );

            DebugNames {
                name: ptr_to_cstr_bytes((*self.ar.get()).name),
                #[cfg(not(feature = "luau"))]
                name_what: ptr_to_cstr_bytes((*self.ar.get()).namewhat),
                #[cfg(feature = "luau")]
                name_what: None,
            }
        }
    }

    /// Corresponds to the `S` what mask.
    pub fn source(&self) -> DebugSource<'lua> {
        unsafe {
            #[cfg(not(feature = "luau"))]
            mlua_assert!(
                ffi::lua_getinfo(self.lua.state, cstr!("S"), self.ar.get()) != 0,
                "lua_getinfo failed with `S`"
            );
            #[cfg(feature = "luau")]
            mlua_assert!(
                ffi::lua_getinfo(self.lua.state, self.level, cstr!("s"), self.ar.get()) != 0,
                "lua_getinfo failed with `s`"
            );

            DebugSource {
                source: ptr_to_cstr_bytes((*self.ar.get()).source),
                #[cfg(not(feature = "luau"))]
                short_src: ptr_to_cstr_bytes((*self.ar.get()).short_src.as_ptr()),
                #[cfg(feature = "luau")]
                short_src: ptr_to_cstr_bytes((*self.ar.get()).short_src),
                line_defined: (*self.ar.get()).linedefined,
                #[cfg(not(feature = "luau"))]
                last_line_defined: (*self.ar.get()).lastlinedefined,
                what: ptr_to_cstr_bytes((*self.ar.get()).what),
            }
        }
    }

    /// Corresponds to the `l` what mask. Returns the current line.
    pub fn curr_line(&self) -> i32 {
        unsafe {
            #[cfg(not(feature = "luau"))]
            mlua_assert!(
                ffi::lua_getinfo(self.lua.state, cstr!("l"), self.ar.get()) != 0,
                "lua_getinfo failed with `l`"
            );
            #[cfg(feature = "luau")]
            mlua_assert!(
                ffi::lua_getinfo(self.lua.state, self.level, cstr!("l"), self.ar.get()) != 0,
                "lua_getinfo failed with `l`"
            );

            (*self.ar.get()).currentline
        }
    }

    /// Corresponds to the `t` what mask. Returns true if the hook is in a function tail call, false
    /// otherwise.
    #[cfg(not(feature = "luau"))]
    #[cfg_attr(docsrs, doc(cfg(not(feature = "luau"))))]
    pub fn is_tail_call(&self) -> bool {
        unsafe {
            mlua_assert!(
                ffi::lua_getinfo(self.lua.state, cstr!("t"), self.ar.get()) != 0,
                "lua_getinfo failed with `t`"
            );
            (*self.ar.get()).currentline != 0
        }
    }

    /// Corresponds to the `u` what mask.
    pub fn stack(&self) -> DebugStack {
        unsafe {
            #[cfg(not(feature = "luau"))]
            mlua_assert!(
                ffi::lua_getinfo(self.lua.state, cstr!("u"), self.ar.get()) != 0,
                "lua_getinfo failed with `u`"
            );
            #[cfg(feature = "luau")]
            mlua_assert!(
                ffi::lua_getinfo(self.lua.state, self.level, cstr!("a"), self.ar.get()) != 0,
                "lua_getinfo failed with `a`"
            );

            #[cfg(not(feature = "luau"))]
            let stack = DebugStack {
                num_ups: (*self.ar.get()).nups as i32,
                #[cfg(any(feature = "lua54", feature = "lua53", feature = "lua52"))]
                num_params: (*self.ar.get()).nparams as i32,
                #[cfg(any(feature = "lua54", feature = "lua53", feature = "lua52"))]
                is_vararg: (*self.ar.get()).isvararg != 0,
            };
            #[cfg(feature = "luau")]
            let stack = DebugStack {
                num_ups: (*self.ar.get()).nupvals as i32,
                num_params: (*self.ar.get()).nparams as i32,
                is_vararg: (*self.ar.get()).isvararg != 0,
            };
            stack
        }
    }
}

enum ActivationRecord {
    #[cfg(not(feature = "luau"))]
    Borrowed(*mut lua_Debug),
    Owned(UnsafeCell<lua_Debug>),
}

impl ActivationRecord {
    #[inline]
    fn get(&self) -> *mut lua_Debug {
        match self {
            #[cfg(not(feature = "luau"))]
            ActivationRecord::Borrowed(x) => *x,
            ActivationRecord::Owned(x) => x.get(),
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
    Unknown(c_int),
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
    #[cfg(not(feature = "luau"))]
    pub last_line_defined: i32,
    pub what: Option<&'a [u8]>,
}

#[derive(Copy, Clone, Debug)]
pub struct DebugStack {
    pub num_ups: i32,
    /// Requires `feature = "lua54/lua53/lua52/luau"`
    #[cfg(any(
        feature = "lua54",
        feature = "lua53",
        feature = "lua52",
        feature = "luau"
    ))]
    pub num_params: i32,
    /// Requires `feature = "lua54/lua53/lua52/luau"`
    #[cfg(any(
        feature = "lua54",
        feature = "lua53",
        feature = "lua52",
        feature = "luau"
    ))]
    pub is_vararg: bool,
}

/// Determines when a hook function will be called by Lua.
#[cfg(not(feature = "luau"))]
#[cfg_attr(docsrs, doc(cfg(not(feature = "luau"))))]
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

#[cfg(not(feature = "luau"))]
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

#[cfg(not(feature = "luau"))]
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

#[cfg(not(feature = "luau"))]
impl BitOrAssign for HookTriggers {
    fn bitor_assign(&mut self, rhs: Self) {
        *self = *self | rhs;
    }
}
