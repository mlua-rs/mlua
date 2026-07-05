//! Lua debugging interface.
//!
//! This module provides access to the Lua debug interface, allowing inspection of the call stack,
//! and function information. The main types are [`struct@Debug`] for accessing debug information
//! and [`HookTriggers`] for configuring debug hooks.

use std::borrow::Cow;
use std::os::raw::c_int;

use ffi::{lua_Debug, lua_State};

use crate::function::Function;
use crate::state::RawLua;
use crate::util::{StackGuard, assert_stack, linenumber_to_usize, ptr_to_lossy_str, ptr_to_str};
#[cfg(feature = "luau")]
use crate::{error::Result, value::Value};

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
                name_what: ptr_to_str((*self.ar).namewhat).filter(|s| !s.is_empty()),
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
    #[cfg(any(feature = "lua55", feature = "lua54", feature = "lua53", feature = "lua52"))]
    #[cfg_attr(
        docsrs,
        doc(cfg(any(feature = "lua55", feature = "lua54", feature = "lua53", feature = "lua52")))
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
                num_upvalues: (*self.ar).nups as _,
                #[cfg(not(any(feature = "lua51", feature = "luajit")))]
                num_params: (*self.ar).nparams as _,
                #[cfg(not(any(feature = "lua51", feature = "luajit")))]
                is_vararg: (*self.ar).isvararg != 0,
            };
            #[cfg(feature = "luau")]
            let stack = DebugStack {
                num_upvalues: (*self.ar).nupvals,
                num_params: (*self.ar).nparams,
                is_vararg: (*self.ar).isvararg != 0,
            };
            stack
        }
    }

    /// Reads local variable `index` (1-based) in this activation record, returning its name and
    /// current value, or `None` once `index` is past the last visible local (wraps `lua_getlocal`).
    ///
    /// Luau keeps locals reachable here even though its sandbox removes `debug.getlocal`, so this is
    /// the way to inspect locals from a [`Lua::set_debug_break`]/[`Lua::set_debug_step`] callback.
    ///
    /// [`Lua::set_debug_break`]: crate::Lua::set_debug_break
    /// [`Lua::set_debug_step`]: crate::Lua::set_debug_step
    #[cfg(any(feature = "luau", doc))]
    #[cfg_attr(docsrs, doc(cfg(feature = "luau")))]
    pub fn get_local(&self, index: usize) -> Option<(String, Value)> {
        unsafe {
            let _sg = StackGuard::new(self.state);
            assert_stack(self.state, 1);

            // `lua_getlocal` pushes the value only when a local exists; otherwise it returns null.
            let name = ptr_to_lossy_str(ffi::lua_getlocal(self.state, self.level, index as c_int))?;
            Some((name.into_owned(), self.lua.pop_value()))
        }
    }

    /// Assigns `value` to local variable `index` (1-based) in this activation record.
    ///
    /// Returns `Ok(Some(name))` with the local's name when the write succeeded, or `Ok(None)` when
    /// `index` is out of range or the frame belongs to a **native-compiled** function (Luau's JIT
    /// guard: `LUA_CALLINFO_NATIVE` prevents writes to native frames because register type tags
    /// could be invalidated).
    ///
    /// For **interpreted** frames the write always succeeds at the C level. If the new value's type
    /// differs from what the Luau bytecode expects, a `RuntimeError` will be raised the next time
    /// the VM touches that register — not here, and not as Rust UB.  This matches the behaviour of
    /// `debug.setlocal` in standard Lua and is intentional: it lets debuggers write any value and
    /// observe the resulting Lua error without compromising memory safety.
    ///
    /// Only meaningful inside a [`Lua::set_debug_break`] / [`Lua::set_debug_step`] callback. For
    /// reliable local visibility, compile the chunk with
    /// [`Compiler::set_optimization_level(0)`][opt] and [`Compiler::set_debug_level(2)`][dbg].
    ///
    /// [opt]: crate::Compiler::set_optimization_level
    /// [dbg]: crate::Compiler::set_debug_level
    /// [`Lua::set_debug_break`]: crate::Lua::set_debug_break
    /// [`Lua::set_debug_step`]: crate::Lua::set_debug_step
    #[cfg(any(feature = "luau", doc))]
    #[cfg_attr(docsrs, doc(cfg(feature = "luau")))]
    pub fn set_local(&self, index: usize, value: Value) -> Result<Option<String>> {
        unsafe {
            // `lua_setlocal` pops the value on success and leaves it on the stack on failure
            // (out-of-range or native frame).  The `StackGuard` restores the top in every case.
            let _sg = StackGuard::new(self.state);
            assert_stack(self.state, 1);

            self.lua.push_value(&value)?;
            let name = ffi::lua_setlocal(self.state, self.level, index as c_int);
            Ok(ptr_to_lossy_str(name).map(|s| s.into_owned()))
        }
    }

    /// Collects every readable local in this record as `(name, value)` pairs, in index order.
    ///
    /// Convenience wrapper over [`Debug::get_local`].
    #[cfg(any(feature = "luau", doc))]
    #[cfg_attr(docsrs, doc(cfg(feature = "luau")))]
    pub fn locals(&self) -> Vec<(String, Value)> {
        let mut locals = Vec::new();
        let mut index = 1;
        while let Some(local) = self.get_local(index) {
            locals.push(local);
            index += 1;
        }
        locals
    }

    /// Returns the name and current value of the `index`-th upvalue (1-based) of the function
    /// running at this stack level. Returns `None` when `index` is out of range.
    ///
    /// The function is pushed then popped internally; the stack is left unchanged.
    pub fn get_upvalue(&self, index: usize) -> Option<(String, Value)> {
        unsafe {
            let _sg = StackGuard::new(self.state);
            assert_stack(self.state, 2); // function + upvalue value

            // Push the function running at this level.
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

            let func_idx = ffi::lua_gettop(self.state);

            // lua_getupvalue pushes the value and returns its name, or returns null and
            // pushes nothing when `index` exceeds the upvalue count.
            let name =
                ptr_to_lossy_str(ffi::lua_getupvalue(self.state, func_idx, index as c_int))?;
            let value = self.lua.pop_value();
            Some((name.into_owned(), value))
            // _sg restores the stack top to before the function push, popping it.
        }
    }

    /// Collects every upvalue of the function at this stack level as `(name, value)` pairs.
    ///
    /// Convenience wrapper over [`Debug::get_upvalue`].
    pub fn upvalues(&self) -> Vec<(String, Value)> {
        let mut upvalues = Vec::new();
        let mut index = 1;
        while let Some(upval) = self.get_upvalue(index) {
            upvalues.push(upval);
            index += 1;
        }
        upvalues
    }
}

/// Represents a specific event that triggered the hook.
#[cfg(not(feature = "luau"))]
#[cfg_attr(docsrs, doc(cfg(not(feature = "luau"))))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DebugEvent {
    Call,
    Ret,
    TailCall,
    Line,
    Count,
    Unknown(c_int),
}

/// Contains the name information of a function in the call stack.
///
/// Returned by the [`Debug::names`] method.
#[derive(Clone, Debug)]
pub struct DebugNames<'a> {
    /// A (reasonable) name of the function (`None` if the name cannot be found).
    pub name: Option<Cow<'a, str>>,
    /// Explains the `name` field (can be `global`/`local`/`method`/`field`/`upvalue`/etc).
    ///
    /// Always `None` for Luau.
    pub name_what: Option<&'static str>,
}

/// Contains the source information of a function in the call stack.
///
/// Returned by the [`Debug::source`] method.
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

/// Contains stack information about a function in the call stack.
///
/// Returned by the [`Debug::stack`] method.
#[derive(Copy, Clone, Debug)]
pub struct DebugStack {
    /// The number of upvalues of the function.
    pub num_upvalues: u8,
    /// The number of parameters of the function (always 0 for C).
    #[cfg(any(not(any(feature = "lua51", feature = "luajit")), doc))]
    #[cfg_attr(docsrs, doc(cfg(not(any(feature = "lua51", feature = "luajit")))))]
    pub num_params: u8,
    /// Whether the function is a variadic function (always true for C).
    #[cfg(any(not(any(feature = "lua51", feature = "luajit")), doc))]
    #[cfg_attr(docsrs, doc(cfg(not(any(feature = "lua51", feature = "luajit")))))]
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
    #[must_use]
    pub const fn on_calls(mut self) -> Self {
        self.on_calls = true;
        self
    }

    /// Returns an instance of `HookTriggers` with [`on_returns`] trigger set.
    ///
    /// [`on_returns`]: #structfield.on_returns
    #[must_use]
    pub const fn on_returns(mut self) -> Self {
        self.on_returns = true;
        self
    }

    /// Returns an instance of `HookTriggers` with [`every_line`] trigger set.
    ///
    /// [`every_line`]: #structfield.every_line
    #[must_use]
    pub const fn every_line(mut self) -> Self {
        self.every_line = true;
        self
    }

    /// Returns an instance of `HookTriggers` with [`every_nth_instruction`] trigger set.
    ///
    /// [`every_nth_instruction`]: #structfield.every_nth_instruction
    #[must_use]
    pub const fn every_nth_instruction(mut self, n: u32) -> Self {
        self.every_nth_instruction = Some(n);
        self
    }

    // Compute the mask to pass to `lua_sethook`.
    #[cfg(not(feature = "luau"))]
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
    #[cfg(not(feature = "luau"))]
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
        self.every_nth_instruction = self.every_nth_instruction.or(rhs.every_nth_instruction);
        self
    }
}

#[cfg(not(feature = "luau"))]
impl std::ops::BitOrAssign for HookTriggers {
    fn bitor_assign(&mut self, rhs: Self) {
        *self = *self | rhs;
    }
}
