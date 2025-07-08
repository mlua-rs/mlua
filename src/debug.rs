use std::borrow::Cow;
use std::os::raw::c_int;

use ffi::{lua_Debug, lua_State};

use crate::function::Function;
use crate::state::RawLua;
use crate::util::{assert_stack, linenumber_to_usize, ptr_to_lossy_str, ptr_to_str, StackGuard};

/// Contains information about currently executing Lua code.
///
/// You may call the methods on this structure to retrieve information about the Lua code executing
/// at the specific level. Further information can be found in the Lua [documentation].
///
/// [documentation]: https://www.lua.org/manual/5.4/manual.html#lua_Debug
pub struct Debug<'a> {
    state: *mut lua_State,
    lua: &'a RawLua,
    #[cfg_attr(not(feature = "luau"), allow(unused))]
    level: c_int,
    ar: *mut lua_Debug,
}

impl<'a> Debug<'a> {
    pub(crate) fn new(lua: &'a RawLua, level: c_int, ar: *mut lua_Debug) -> Self {
        Debug {
            state: lua.state(),
            lua,
            ar,
            level,
        }
    }

    /// Returns the specific event that triggered the hook.
    ///
    /// For [Lua 5.1] [`DebugEvent::TailCall`] is used for return events to indicate a return
    /// from a function that did a tail call.
    ///
    /// [Lua 5.1]: https://www.lua.org/manual/5.1/manual.html#pdf-LUA_HOOKTAILRET
    #[cfg(not(feature = "luau"))]
    #[cfg_attr(docsrs, doc(cfg(not(feature = "luau"))))]
    pub fn event(&self) -> DebugEvent {
        unsafe {
            match (*self.ar).event {
                ffi::LUA_HOOKCALL => DebugEvent::Call,
                ffi::LUA_HOOKRET => DebugEvent::Ret,
                ffi::LUA_HOOKTAILCALL => DebugEvent::TailCall,
                ffi::LUA_HOOKLINE => DebugEvent::Line,
                ffi::LUA_HOOKCOUNT => DebugEvent::Count,
                event => DebugEvent::Unknown(event),
            }
        }
    }

    /// Returns the function that is running at the given level.
    ///
    /// Corresponds to the `f` "what" mask.
    pub fn function(&self) -> Function {
        unsafe {
            let _sg = StackGuard::new(self.state);
            assert_stack(self.state, 1);

            #[cfg(not(feature = "luau"))]
            mlua_assert!(
                ffi::lua_getinfo(self.state, cstr!("f"), self.ar) != 0,
                "lua_getinfo failed with `f`"
            );
            #[cfg(feature = "luau")]
            mlua_assert!(
                ffi::lua_getinfo(self.state, self.level, cstr!("f"), self.ar) != 0,
                "lua_getinfo failed with `f`"
            );

            ffi::lua_xmove(self.state, self.lua.ref_thread(), 1);
            Function(self.lua.pop_ref_thread())
        }
    }

    /// Corresponds to the `n` "what" mask.
    pub fn names(&self) -> DebugNames<'_> {
        unsafe {
            #[cfg(not(feature = "luau"))]
            mlua_assert!(
                ffi::lua_getinfo(self.state, cstr!("n"), self.ar) != 0,
                "lua_getinfo failed with `n`"
            );
            #[cfg(feature = "luau")]
            mlua_assert!(
                ffi::lua_getinfo(self.state, self.level, cstr!("n"), self.ar) != 0,
                "lua_getinfo failed with `n`"
            );

            DebugNames {
                name: ptr_to_lossy_str((*self.ar).name),
                #[cfg(not(feature = "luau"))]
                name_what: match ptr_to_str((*self.ar).namewhat) {
                    Some("") => None,
                    val => val,
                },
                #[cfg(feature = "luau")]
                name_what: None,
            }
        }
    }

    /// Corresponds to the `S` "what" mask.
    pub fn source(&self) -> DebugSource<'_> {
        unsafe {
            #[cfg(not(feature = "luau"))]
            mlua_assert!(
                ffi::lua_getinfo(self.state, cstr!("S"), self.ar) != 0,
                "lua_getinfo failed with `S`"
            );
            #[cfg(feature = "luau")]
            mlua_assert!(
                ffi::lua_getinfo(self.state, self.level, cstr!("s"), self.ar) != 0,
                "lua_getinfo failed with `s`"
            );

            DebugSource {
                source: ptr_to_lossy_str((*self.ar).source),
                #[cfg(not(feature = "luau"))]
                short_src: ptr_to_lossy_str((*self.ar).short_src.as_ptr()),
                #[cfg(feature = "luau")]
                short_src: ptr_to_lossy_str((*self.ar).short_src),
                line_defined: linenumber_to_usize((*self.ar).linedefined),
                #[cfg(not(feature = "luau"))]
                last_line_defined: linenumber_to_usize((*self.ar).lastlinedefined),
                #[cfg(feature = "luau")]
                last_line_defined: None,
                what: ptr_to_str((*self.ar).what).unwrap_or("main"),
            }
        }
    }

    #[doc(hidden)]
    #[deprecated(note = "Use `current_line` instead")]
    pub fn curr_line(&self) -> i32 {
        self.current_line().map(|n| n as i32).unwrap_or(-1)
    }

    /// Corresponds to the `l` "what" mask. Returns the current line.
    pub fn current_line(&self) -> Option<usize> {
        unsafe {
            #[cfg(not(feature = "luau"))]
            mlua_assert!(
                ffi::lua_getinfo(self.state, cstr!("l"), self.ar) != 0,
                "lua_getinfo failed with `l`"
            );
            #[cfg(feature = "luau")]
            mlua_assert!(
                ffi::lua_getinfo(self.state, self.level, cstr!("l"), self.ar) != 0,
                "lua_getinfo failed with `l`"
            );

            linenumber_to_usize((*self.ar).currentline)
        }
    }

    /// Corresponds to the `t` "what" mask. Returns true if the hook is in a function tail call,
    /// false otherwise.
    #[cfg(any(feature = "lua54", feature = "lua53", feature = "lua52"))]
    #[cfg_attr(
        docsrs,
        doc(cfg(any(feature = "lua54", feature = "lua53", feature = "lua52")))
    )]
    pub fn is_tail_call(&self) -> bool {
        unsafe {
            mlua_assert!(
                ffi::lua_getinfo(self.state, cstr!("t"), self.ar) != 0,
                "lua_getinfo failed with `t`"
            );
            (*self.ar).istailcall != 0
        }
    }

    /// Corresponds to the `u` "what" mask.
    pub fn stack(&self) -> DebugStack {
        unsafe {
            #[cfg(not(feature = "luau"))]
            mlua_assert!(
                ffi::lua_getinfo(self.state, cstr!("u"), self.ar) != 0,
                "lua_getinfo failed with `u`"
            );
            #[cfg(feature = "luau")]
            mlua_assert!(
                ffi::lua_getinfo(self.state, self.level, cstr!("au"), self.ar) != 0,
                "lua_getinfo failed with `au`"
            );

            #[cfg(not(feature = "luau"))]
            let stack = DebugStack {
                num_ups: (*self.ar).nups as _,
                #[cfg(any(feature = "lua54", feature = "lua53", feature = "lua52"))]
                num_params: (*self.ar).nparams as _,
                #[cfg(any(feature = "lua54", feature = "lua53", feature = "lua52"))]
                is_vararg: (*self.ar).isvararg != 0,
            };
            #[cfg(feature = "luau")]
            let stack = DebugStack {
                num_ups: (*self.ar).nupvals,
                num_params: (*self.ar).nparams,
                is_vararg: (*self.ar).isvararg != 0,
            };
            stack
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
    /// A (reasonable) name of the function (`None` if the name cannot be found).
    pub name: Option<Cow<'a, str>>,
    /// Explains the `name` field (can be `global`/`local`/`method`/`field`/`upvalue`/etc).
    ///
    /// Always `None` for Luau.
    pub name_what: Option<&'static str>,
}

#[derive(Clone, Debug)]
pub struct DebugSource<'a> {
    /// Source of the chunk that created the function.
    pub source: Option<Cow<'a, str>>,
    /// A "printable" version of `source`, to be used in error messages.
    pub short_src: Option<Cow<'a, str>>,
    /// The line number where the definition of the function starts.
    pub line_defined: Option<usize>,
    /// The line number where the definition of the function ends (not set by Luau).
    pub last_line_defined: Option<usize>,
    /// A string `Lua` if the function is a Lua function, `C` if it is a C function, `main` if it is
    /// the main part of a chunk.
    pub what: &'static str,
}

#[derive(Copy, Clone, Debug)]
pub struct DebugStack {
    /// Number of upvalues.
    pub num_ups: u8,
    /// Number of parameters.
    #[cfg(any(feature = "lua54", feature = "lua53", feature = "lua52", feature = "luau"))]
    #[cfg_attr(
        docsrs,
        doc(cfg(any(feature = "lua54", feature = "lua53", feature = "lua52", feature = "luau")))
    )]
    pub num_params: u8,
    /// Whether the function is a vararg function.
    #[cfg(any(feature = "lua54", feature = "lua53", feature = "lua52", feature = "luau"))]
    #[cfg_attr(
        docsrs,
        doc(cfg(any(feature = "lua54", feature = "lua53", feature = "lua52", feature = "luau")))
    )]
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
    /// An instance of `HookTriggers` with `on_calls` trigger set.
    pub const ON_CALLS: Self = HookTriggers::new().on_calls();

    /// An instance of `HookTriggers` with `on_returns` trigger set.
    pub const ON_RETURNS: Self = HookTriggers::new().on_returns();

    /// An instance of `HookTriggers` with `every_line` trigger set.
    pub const EVERY_LINE: Self = HookTriggers::new().every_line();

    /// Returns a new instance of `HookTriggers` with all triggers disabled.
    pub const fn new() -> Self {
        HookTriggers {
            on_calls: false,
            on_returns: false,
            every_line: false,
            every_nth_instruction: None,
        }
    }

    /// Returns an instance of `HookTriggers` with [`on_calls`] trigger set.
    ///
    /// [`on_calls`]: #structfield.on_calls
    pub const fn on_calls(mut self) -> Self {
        self.on_calls = true;
        self
    }

    /// Returns an instance of `HookTriggers` with [`on_returns`] trigger set.
    ///
    /// [`on_returns`]: #structfield.on_returns
    pub const fn on_returns(mut self) -> Self {
        self.on_returns = true;
        self
    }

    /// Returns an instance of `HookTriggers` with [`every_line`] trigger set.
    ///
    /// [`every_line`]: #structfield.every_line
    pub const fn every_line(mut self) -> Self {
        self.every_line = true;
        self
    }

    /// Returns an instance of `HookTriggers` with [`every_nth_instruction`] trigger set.
    ///
    /// [`every_nth_instruction`]: #structfield.every_nth_instruction
    pub const fn every_nth_instruction(mut self, n: u32) -> Self {
        self.every_nth_instruction = Some(n);
        self
    }

    // Compute the mask to pass to `lua_sethook`.
    pub(crate) const fn mask(&self) -> c_int {
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
    pub(crate) const fn count(&self) -> c_int {
        match self.every_nth_instruction {
            Some(n) => n as c_int,
            None => 0,
        }
    }
}

#[cfg(not(feature = "luau"))]
impl std::ops::BitOr for HookTriggers {
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
impl std::ops::BitOrAssign for HookTriggers {
    fn bitor_assign(&mut self, rhs: Self) {
        *self = *self | rhs;
    }
}
