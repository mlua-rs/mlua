## v0.7.3

- Fixed cross-compilation issue (introduced in 84a174c)

## v0.7.2

- Allow `pkg-config` to omit include paths if they equals to standard (#114).
- Various bugfixes (eg. #121)

## v0.7.1

- Fixed traceback generation for errors (#112)
- `Lua::into_static/from_static` methods have been removed from the docs and are discouraged for use

## v0.7.0

- New "application data" api to store arbitrary objects inside Lua
- New feature flag `luajit52` to build/support LuaJIT with partial compatibility with Lua 5.2
- Added async meta methods for all Lua (except 5.1)
- Added `AnyUserData::take()` to take UserData objects from Lua
- Added `set_nth_user_value`/`get_nth_user_value` to `AnyUserData` for all Lua versions
- Added `set_named_user_value`/`get_named_user_value` to `AnyUserData` for all Lua versions
- Added `Lua::inspect_stack()` to get information about the interpreter runtime stack
- Added `set_warning_function`/`remove_warning_function`/`warning` functions to `Lua` for 5.4
- Added `TableExt::call()` to call tables with `__call` metamethod as functions
- Added `Lua::unload()` to unload modules
- `ToLua` implementation for arrays changed to const generics
- Added thread (coroutine) cache for async execution (disabled by default and works for Lua 5.4/JIT)
- LuaOptions and (De)SerializeOptions marked as const
- Fixed recursive tables serialization when using `serde::Serialize` for Lua Tables
- Improved errors reporting. Now source included to `fmt::Display` implementation for `Error::CallbackError`
- Major performance improvements

## v0.6.6

- Fixed calculating `LUA_REGISTRYINDEX` when cross-compiling for lua51/jit (#82)
- Updated documentation & examples

## v0.6.5

- Fixed bug when polling async futures (#77)
- Refactor Waker handling in async code (+10% performance gain when calling async functions)
- Added `Location::caller()` information to `Lua::load()` if chunk's name is None (Rust 1.46+)
- Added serialization of i128/u128 types (serde)

## v0.6.4

- Performance optimizations
- Fixed table traversal used in recursion detection in deserializer

## v0.6.3

- Disabled catching Rust panics in userdata finalizers on drop. It also has positive performance impact.
- Added `Debug::event()` to the hook's Debug structure
- Simplified interface of `hook::HookTriggers`
- Added finalizer to `ExtraData` in module mode. This helps avoiding memory leak on closing state when Lua unloads modules and frees memory.
- Added `DeserializeOptions` struct to control deserializer behavior (`from_value_with` function).

## v0.6.2

- New functionality: `Lua::load_from_function()` and `Lua::create_c_function()`
- Many optimizations in callbacks/userdata creation and methods execution

## v0.6.1

- Update `chunk!` documentation (stable Rust limitations)
- Fixed Lua sequence table conversion to HashSet/BTreeSet
- `once_cell` dependency lowered to 1.0

## v0.6.0
Changes since 0.5.4
- New `UserDataFields` API
- Full access to `UserData` metatables with support of setting arbitrary fields.
- Implement `UserData` for `Rc<RefCell<T>>`/`Arc<Mutex<T>>`/`Arc<RwLock<T>>` where `T: UserData`.
- Added `SerializeOptions` to to change default Lua serializer behaviour (eg. `nil/null/array` serialization)
- Added `LuaOptions` to customize Lua/Rust behaviour (currently panic handling)
- Added `ToLua`/`FromLua` implementation for `Box<str>` and `Box<[T]>`.
- Added `Thread::reset()` for luajit/lua54 to recycle threads (coroutines) with attaching a new function.
- Added `chunk!` macro support to load chunks of Lua code using the Rust tokenizer and optionally capturing Rust variables.
- Improved errors reporting (`Error`'s `__tostring` method formats full stacktraces). This is useful in a module mode.
- Added `String::to_string_lossy`
- Various bugfixes and improvements

Breaking changes:
- Errors are always `Send + Sync` to be compatible with the anyhow crate.
- Removed `Result` from `LuaSerdeExt::null()` and `LuaSerdeExt::array_metatable()` (never fails)
- Removed `Result` from `Function::dump()` (never fails)
- Removed `AnyUserData::has_metamethod()` (in favour of full access to metatables)

## v0.6.0-beta.3

- Errors are always `Send + Sync` to be compatible with anyhow crate
- Implement `UserData` for `Rc<RefCell>`/`Arc<Mutex>`/`Arc<RwLock>`
- Added `__ipairs` metamethod for Lua 5.2
- Added `String::to_string_lossy`
- Various bugfixes and improvements

## v0.6.0-beta.2

- [**Breaking**] Removed `AnyUserData::has_metamethod()`
- Added `Thread::reset()` for luajit/lua54 to recycle threads.
  It's possible to attach a new function to a thread (coroutine).
- Added `chunk!` macro support to load chunks of Lua code using the Rust tokenizer and optinally capturing Rust variables.
- Improved error reporting (`Error`'s `__tostring` method formats full stacktraces). This is useful in the module mode.

## v0.6.0-beta.1

- New `UserDataFields` API
- Allow to define arbitrary MetaMethods
- `MetaMethods::name()` is public
- Do not trigger longjmp in Rust to prevent unwinding across FFI boundaries. See https://github.com/rust-lang/rust/issues/83541
- Added `SerializeOptions` to to change default Lua serializer behaviour (eg. nil/null/array serialization)
- [**Breaking**] Removed `Result` from `LuaSerdeExt::null()` and `LuaSerdeExt::array_metatable()` (never fails)
- [**Breaking**] Removed `Result` from `Function::dump()` (never fails)
- `ToLua`/`FromLua` implementation for `Box<str>` and `Box<[T]>`
- [**Breaking**] Added `LuaOptions` to customize Lua/Rust behaviour (currently panic handling)
- Various bugfixes and performance improvements

## v0.5.4

- Build script improvements
- Improvements in panic handling (resume panic on value popping)
- Fixed bug serializing 3rd party userdata (causes segfault)
- Make error::Error non exhaustive

## v0.5.3

- Fixed bug when returning nil-prefixed multi values from async function (+ test)
- Performance optimisation for async callbacks (polling)

## v0.5.2

- Some performance optimisations (callbacks)
- `ToLua` implementation for `Cow<str>` and `Cow<CStr>`
- Fixed bug with `Scope` destruction of partially polled futures

## v0.5.1

- Support cross compilation that should work well for vendored builds (including LuaJIT with some restrictions)
- Fix numeric types conversion for 32bit Lua
- Update tokio to 1.0 for async examples

## v0.5.0

- Serde support under `serialize` feature flag.
- Re-export `mlua_derive`.
- impl `ToLua` and `FromLua` for `HashSet` and `BTreeSet`

## v0.4.2

- Added `Function::dump()` to dump lua function to a binary chunk
- Added `ChunkMode` enum to mark chunks as text or binary
- Updated `set_memory_limit` doc

## v0.4.0

- Lua 5.4 support with `MetaMethod::Close`.
- `lua53` feature is disabled by default. Now preferred Lua version have to be chosen explicitly.
- Provide safety guaraness for Lua state, which means that potenially unsafe operations, like loading C modules (using `require` or `package.loadlib`) are disabled. Equalient for the previous `Lua::new()` function is `Lua::unsafe_new()`.
- New `send` feature to require `Send`.
- New `module` feature, that disables linking to Lua Core Libraries. Required for modules.
- Don't allow `'callback` outlive `'lua` in `Lua::create_function()` to fix [the unsoundness](tests/compile/static_callback_args.rs).
- Added `Lua::into_static()` to make `'static` Lua state. This is useful to spawn async Lua threads that requires `'static`.
- New function `Lua::set_memory_limit()` (similar to `rlua`) to enable memory restrictions in Lua VM (requires Lua >= 5.2).
- `Scope`, temporary removed in v0.3, is back with async support.
- Removed deprecated `Table::call()` function.
- Added hooks support (backported from rlua 0.17).
- New `AnyUserData::has_metamethod()` function.
- LuaJIT 2.0.5 (the latest stable) support.
- Various bug fixes and improvements.
