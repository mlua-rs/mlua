## v0.11.1 (Jul 15, 2025)

- Fixed bug exhausting Lua auxiliary stack and leaving it without reserve (#615)
- `Lua::push_c_function` now correctly handles OOM for Lua 5.1 and Luau

## v0.11.0 (Jul 14, 2025)

Changes since v0.11.0-beta.3

- Allow linking external Lua libraries in a build script (e.g. pluto) using `external` mlua-sys feature flag
- `Lua::inspect_stack` takes a callback with `&Debug` argument, instead of returning `Debug` directly
- Added `Debug::function` method to get function running at a given level
- `Debug::curr_line` is deprecated in favour of `Debug::current_line` that returns `Option<usize>`
- Added `Lua::set_globals` method to replace global environment
- `Table::set_metatable` now returns `Result<()>` (this operation can fail in sandboxed Luau mode)
- `impl ToString` replaced with `Into<StdString>`  in `UserData` registration
- `Value::as_str` and `Value::as_string_lossy` methods are deprecated (as they are non-idiomatic)
- Bugfixes and improvements

## v0.11.0-beta.3 (Jun 23, 2025)

- Luau in sandboxed mode has reduced options in `collectgarbage` function (to follow the official doc)
- `Function::deep_clone` now returns `Result<Function>` as this operation can trigger memory errors
- Luau "Require" resolves included Lua files relative to the current directory (#605)
- Fixed bug when finalizing `AsyncThread` on drop (`call_async` methods family)

## v0.11.0-beta.2 (Jun 12, 2025)

- Lua 5.4 updated to 5.4.8
- Terminate Rust `Future` when `AsyncThread` is dropped (without relying on Lua GC)
- Added `loadstring` function to Luau
- Make `AsChunk` trait dyn-friendly
- Luau `Require` trait synced with Luau 0.674
- Luau `Require` trait methods now can return `Error` variant (in `NavigateError` enum)
- Added `__type` to `Error`'s userdata metatable (for `typeof` function)
- `parking_log/send_guard` is moved to `userdata-wrappers` feature flag
- New `serde` feature flag to replace `serialize` (the old one is still available)

## v0.11.0-beta.1 (May 7th, 2025)

- New "require-by-string" for Luau (with `Require` trait and async support)
- Added `Thread::resume_error` support for Luau
- 52 bit integers support for Luau (this is a breaking change)
- New features for Luau compiler (constants, disabled builtins, known members)
- `AsyncThread<A, R>` changed to `AsyncThread<R>` (`A` pushed to stack immediately)
- Lifetime `'a` moved from `AsChunk<'a>` to `AsChunk::source where Self: 'a`
- `Lua::scope` pass `&Scope` instead of `&mut Scope` to closure
- Added global hooks support (Lua 5.1+)
- Added per-thread hooks support (Lua 5.1+)
- `Lua::init_from_ptr` renamed to `Lua::get_or_init_from_ptr` and returns `&Lua`
- `Lua:load_from_function` is deprecated (this is `register_module` now)
- Added `Lua::register_module` and `Lua::preload_module`

## v0.10.4 (May 5th, 2025)

- Luau updated to 0.672
- New serde option `encode_empty_tables_as_array` to serialize empty tables as arrays
- Added `WeakLua` and `Lua::weak()` to create weak references to Lua state
- Trigger abort when Luau userdata destructors are panic (Luau GC does not support it)
- Added `AnyUserData::type_id()` method to get the type id of the userdata
- Added `Chunk::name()`, `Chunk::environment()` and `Chunk::mode()` functions
- Support borrowing underlying wrapped types for `UserDataRef` and `UserDataRefMut` (under `userdata-wrappers` feature)
- Added large (52bit) integers support for Luau
- Enable `serde` for `bstr` if `serialize` feature flag is enabled
- Recursive warnings (Lua 5.4) are no longer allowed
- Implemented `IntoLua`/`FromLua` for `BorrowedString` and `BorrowedBytes`
- Implemented `IntoLua`/`FromLua` for `char`
- Enable `Thread::reset()` for all Lua versions (limited support for 5.1-5.3)
- Bugfixes and improvements

## v0.10.3 (Jan 27th, 2025)

- Set `Default` for `Value` to be `Nil`
- Allow exhaustive match on `Value` (#502)
- Add `Table::set_safeenv` method (Luau)

## v0.10.2 (Dec 1st, 2024)

- Switch proc-macro-error to proc-macro-error2 (#493)
- Do not allow Lua to run GC finalizers on ref thread (#491)
- Fix chunks loading in Luau when memory limit is enforced (#488)
- Added `String::wrap` method to wrap arbitrary `AsRef<[u8]>` into `impl IntoLua`
- Better FreeBSD/OpenBSD support (thanks to cos)
- Delay "any" userdata metatable creation until first instance is created (#482)
- Reduce amount of generated code for `UserData` (less generics)

## v0.10.1 (Nov 9th, 2024)

- Minimal Luau updated to 0.650
- Added Luau native vector library support (this can change behavior if you use `vector` function!)
- Added Lua `String::display` method
- Improved pretty-printing for Lua tables (#478)
- Added `Scope::create_any_userdata` to create Lua objects from any non-`'static` Rust types
- Added `AnyUserData::destroy` method
- New `userdata-wrappers` feature to `impl UserData` for `Rc<T>`/`Arc<T>`/`Rc<RefCell<T>>`/`Arc<Mutex<T>>` (similar to v0.9)
- `UserDataRef` in `send` mode now uses shared lock if `T: Sync` (and exclusive lock otherwise)
- Added `Scope::add_destructor` to attach custom destructors
- Added `Lua::try_app_data_ref` and `Lua::try_app_data_mut` methods
- Added `From<Vec>` and `Into<Vec>` support to `MultiValue` and `Variadic` types
- Bug fixes and improvements (#477 #479)

## v0.10.0 (Oct 25th, 2024)

Changes since v0.10.0-rc.1

- Added `error-send` feature flag (disabled by default) to require `Send + Sync` for `Error`
- Some performance improvements

## v0.10.0-rc.1

- `Lua::scope` is back
- Support yielding from hooks for Lua 5.3+
- Support setting metatable for Lua builtin types (number/string/function/etc)
- Added `LuaNativeFn`/`LuaNativeFnMut`/`LuaNativeAsyncFn` traits for using in `Function::wrap`
- Added `Error::chain` method to return iterator over nested errors
- Added `Lua::exec_raw` helper to execute low-level Lua C API code
- Added `Either<L, R>` enum to combine two types into a single one
- Added a new `Buffer` type for Luau
- Added `Value::is_error` and `Value::as_error` helpers
- Added `Value::Other` variant to represent unknown Lua types (eg LuaJIT CDATA)
- Added (optional) `anyhow` feature to implement `IntoLua` for `anyhow::Error`
- Added `IntoLua`/`FromLua` for `OsString`/`OsStr` and `PathBuf`/`Path`

## v0.10.0-beta.2

- Updated `ThreadStatus` enum to include `Running` and `Finished` variants.
- `Error::CoroutineInactive` renamed to `Error::CoroutineUnresumable`.
- `IntoLua`/`IntoLuaMulti` now uses `impl trait` syntax for args (shorten from `a.get::<_, T>` to `a.get::<T>`).
- Removed undocumented `Lua::into_static`/`from_static` methods.
- Futures now require `Send` bound if `send` feature is enabled.
- Dropped lifetime from `UserDataMethods` and `UserDataFields` traits.
- `Compiler::compile()` now returns `Result` (Luau).
- Removed `Clone` requirement from `UserDataFields::add_field()`.
- `TableExt` and `AnyUserDataExt` traits were combined into `ObjectLike` trait.
- Disabled `send` feature in module mode (since we don't have exclusive access to Lua).
- `Chunk::set_environment()` takes `Table` instead of `IntoLua` type.
- Reduced the compile time contribution of `next_key_seed` and `next_value_seed`.
- Reduced the compile time contribution of `serde_userdata`.
- Performance improvements.

## v0.10.0-beta.1

- Dropped `'lua` lifetime (subtypes now store a weak reference to Lua)
- Removed (experimental) owned types (they no longer needed)
- Make Lua types truly `Send` and `Sync` (when enabling `send` feature flag)
- Removed `UserData` impl for Rc/Arc types ("any" userdata functions can be used instead)
- `Lua::replace_registry_value` takes `&mut RegistryKey`
- `Lua::scope` temporary disabled (will be re-added in the next release)

## v0.9.9

- Minimal Luau updated to 0.629
- Fixed bug when attempting to reset or resume already running coroutines (#416).
- Added `RegistryKey::id()` method to get the underlying Lua registry key id.

## v0.9.8

- Fixed serializing same table multiple times (#408)
- Use `mlua-sys` v0.6 (to support Luau 0.624+)
- Fixed cross compilation of windows dlls from unix (#394)

## v0.9.7

- Implemented `IntoLua` for `RegistryKey`
- Mark `__idiv` metamethod as available for luau
- Added `Function::deep_clone()` method (Luau)
- Added `SerializeOptions::detect_serde_json_arbitrary_precision` option
- Added `Lua::create_buffer()` method (Luau)
- Support serializing buffer type as a byte slice (Luau)
- Perf: Implemented `push_into_stack`/`from_stack` for `Option<T>`
- Added `Lua::create_ser_any_userdata()` method

## v0.9.6

- Added `to_pointer` function to `Function`/`Table`/`Thread`
- Implemented `IntoLua` for `&Value`
- Implemented `FromLua` for `RegistryKey`
- Faster (~5%) table array traversal during serialization
- Some performance improvements for bool/int types

## v0.9.5

- Minimal Luau updated to 0.609
- Luau max stack size increased to 1M (from 100K)
- Implemented `IntoLua` for refs to `String`/`Table`/`Function`/`AnyUserData`/`Thread` + `RegistryKey`
- Implemented `IntoLua` and `FromLua` for `OwnedThread`/`OwnedString`
- Fixed `FromLua` derive proc macro to cover more cases

## v0.9.4

- Fixed loading all-in-one modules under mixed states (eg. main state and coroutines)

## v0.9.3

- WebAssembly support (`wasm32-unknown-emscripten` target)
- Performance improvements (faster Lua function calls for lua51/jit/luau)

## v0.9.2

- Added binary modules support to Luau
- Added Luau package module (uses `StdLib::PACKAGE`) with loaders (follows lua5.1 interface)
- Added support of Luau 0.601+ buffer type (represented as userdata in Rust)
- LuaJIT `cdata` type is also represented as userdata in Rust (instead of panic)
- Vendored LuaJIT switched to rolling vanilla (from openresty)
- Added `Table::for_each` method for fast table pairs traversal (faster than `pairs`)
- Performance improvements around table traversal (and faster serialization)
- Bug fixes and improvements

## v0.9.1

- impl Default for Lua
- impl IntoLuaMulti for `std::result::Result<(), E>`
- Fix using wrong userdata index after processing Variadic args (#311)

## v0.9.0

Changes since v0.9.0-rc.3

- Improved non-static (scoped) userdata support
- Added `Scope::create_any_userdata()` method
- Added `Lua::set_vector_metatable()` method (`unstable` feature flag)
- Added `OwnedThread` type (`unstable` feature flag)
- Minimal Luau updated to 0.590
- Added new option `sort_keys` to `DeserializeOptions` (`Lua::from_value()` method)
- Changed `Table::raw_len()` output type to `usize`
- Helper functions for `Value` (eg: `Value::as_number()`/`Value::as_string`/etc)
- Performance improvements

## v0.9.0-rc.3

- Minimal Luau updated to 0.588

## v0.9.0-rc.2

- Added `#[derive(FromLua)]` macro to opt-in into `FromLua<T> where T: 'static + Clone` (userdata type).
- Support vendored module mode for windows (raw-dylib linking, Rust 1.71+)
- `module` and `vendored` features are now mutually exclusive
- Use `C-unwind` ABI (Rust 1.71+)
- Changed `AsChunk` trait to support capturing wrapped Lua types

## v0.9.0-rc.1

- `UserDataMethods::add_async_method()` takes `&T` instead of cloning `T`
- Implemented `PartialEq<[T]>` for tables
- Added Luau 4-dimensional vectors support (`luau-vector4` feature)
- `Table::sequence_values()` iterator no longer uses any metamethods (`Table::raw_sequence_values()` is deprecated)
- Added `Table:is_empty()` function that checks both hash and array parts
- Refactored Debug interface
- Re-exported `ffi` (`mlua-sys`) crate for easier writing of unsafe code
- Refactored Lua 5.4 warnings interface
- Take `&str` as function name in `TableExt` and `AnyUserDataExt` traits
- Added module attribule `skip_memory_check` to improve performance
- Added `AnyUserData::wrap()` to provide more easy way of creating _any_ userdata in Lua

## v0.9.0-beta.3

- Added `OwnedAnyUserData::take()`
- Switch to `DeserializeOwned`
- Overwrite error context when called multiple times
- New feature flag `luau-jit` to enable (experimental) Luau codegen backend
- Set `__name` field in userdata metatable
- Added `Value::to_string()` method similar to `luaL_tolstring`
- Lua 5.4.6
- Application data container now allows to mutably and immutably borrow different types at the same time
- Performance optimizations
- Support getting and setting environment for Lua functions.
- Added `UserDataFields::add_field()` method to add static fields to UserData

Breaking changes:
- Require environment to be a `Table` instead of `Value` in Chunks.
- `AsChunk::env()` renamed to `AsChunk::environment()`

## v0.9.0-beta.2

New features:
- Added `Thread::set_hook()` function to set hook on threads
- Added pretty print to the Debug formatting to Lua `Value` and `Table`
- ffi layer moved to `mlua-sys` crate
- Added OwnedString (unstable)

Breaking changes:
- Refactor `HookTriggers` (make it const)

## v0.9.0-beta.1

New features:
- Owned Lua types (unstable feature flag)
- New functions `Function::wrap`/`Function::wrap_mut`/`Function::wrap_async`
- `Lua::register_userdata_type()` to register a custom userdata types (without requiring `UserData` trait)
- `Lua::create_any_userdata()`
- Added `create_userdata_ref`/`create_userdata_ref_mut` for scopes
- Added `AnyUserDataExt` trait with auxiliary functions for `AnyUserData`
- Added `UserDataRef` and `UserDataRefMut` type wrapped that implement `FromLua`
- Improved error handling:
  * Improved error reporting when calling Rust functions from Lua.
  * Added `Error::BadArgument` to help identify bad argument position or name
  * Added `ErrorContext` extension trait to attach additional context to `Error`

Breaking changes:
- Refactored `AsChunk` trait
- `ToLua`/`ToLuaMulti` renamed to `IntoLua`/`IntoLuaMulti`
- Renamed `to_lua_err` to `into_lua_err`
- Removed `FromLua` impl for `T: UserData+Clone`
- Removed `Lua::async_scope`
- Added `&Lua` arg to Luau interrupt callback

Other:
- Better Debug for String
- Allow deserializing values from serializable UserData using `Lua::from_value()` method
- Added `Table::clear()` method
- Added `Error::downcast_ref()` method
- Support setting memory limit for Lua 5.1/JIT/Luau
- Support setting module name in `#[lua_module(name = "...")]` macro
- Minor fixes and improvements

## v0.8.10

- Update to Luau 0.590 (luau0-src to 0.7.x)
- Fix loading luau code starting with \t
- Pin lua-src and luajit-src versions

## v0.8.9

- Update minimal (vendored) Lua 5.4 to 5.4.6
- Use `lua_closethread` instead of `lua_resetthread` in vendored mode (Lua 5.4.6)
- Allow deserializing Lua null into unit (`()`) or unit struct.

## v0.8.8

- Fix potential deadlock when trying to reuse dropped registry keys.
- Optimize userdata methods call when __index and fields_getters are nil

## v0.8.7

- Minimum Luau updated to 0.555 (`LUAI_MAXCSTACK` limit increased to 100000)
- `_VERSION` in Luau now includes version number
- Fixed lifetime of `DebugNames` in `Debug::names()` and `DebugSource` in `Debug::source()`
- Fixed subtraction overflow when calculating index for `MultiValue::get()`

## v0.8.6

- Fixed bug when recycled Registry slot can be set to Nil

## v0.8.5

- Fixed potential unsoundness when using `Layout::from_size_align_unchecked` and Rust 1.65+
- Performance optimizations around string and table creation in standalone mode
- Added fast track path to Table `get`/`set`/`len` methods without metatable
- Added new methods `push`/`pop`/`raw_push`/`raw_pop` to Table
- Fix getting caller information from `Lua::load`
- Better checks and tests when trying to modify a Luau readonly table

## v0.8.4

- Minimal Luau updated to 0.548

## v0.8.3

- Close to-be-closed variables for Lua 5.4 when using call_async functions (#192)
- Fixed Lua assertion when inspecting another thread stack. (#195)
- Use more reliable way to create LuaJIT VM (which can fail if use Rust allocator on non-x86 platforms)

## v0.8.2

- Performance optimizations in handling UserData
- Minimal Luau updated to 0.536
- Fixed bug in `Function::bind` when passing empty binds and no arguments (#189)

## v0.8.1

- Added `Lua::create_proxy` for accessing to UserData static fields and functions without instance
- Added `Table::to_pointer()` and `String::to_pointer()` functions
- Bugfixes and improvements (#176 #179)

## v0.8.0
Changes since 0.7.4
- Luau support
- Removed C glue
- Added async support to `__index` and `__newindex` metamethods
- Added `Function::info()` to get information about functions (#149).
- Added `parking_lot` dependency under feature flag (for `UserData`)
- `Hash` implementation for Lua String
- Added `Value::to_pointer()` function
- Performance improvements

Breaking changes:
- Refactored `AsChunk` trait (added implementation for `Path` and `PathBuf`).

## v0.8.0-beta.5

- Lua sources no longer needed to build modules
- Added `__iter` metamethod for Luau
- Added `Value::to_pointer()` function
- Added `Function::coverage` for Luau to obtain coverage report
- Bugfixes and improvements (#153 #161 #168)

## v0.8.0-beta.4

- Removed `&Lua` from `Lua::set_interrupt` as it's not safe (introduced in v0.8.0-beta.3)
- Enabled `Lua::gc_inc` for Luau
- Luau `debug` module marked as safe (enabled by default)
- Implemented `Hash` for Lua String
- Support mode options in `collectgarbage` for Luau
- Added ability to set global Luau compiler (used for loading all chunks).
- Refactored `AsChunk` trait (breaking changes).
  `AsChunk` now implemented for `Path` and `PathBuf` to load lua files from fs.
- Added `parking_lot` dependency and feature flag (for `UserData`)
- Added `Function::info()` to get information about functions (#149).
- Bugfixes and improvements (#104 #142)

## v0.8.0-beta.3

- Luau vector constructor
- Luau sandboxing support
- Luau interrupts (yieldable)
- More Luau compiler options (mutable globals)
- Other performance improvements

## v0.8.0-beta.2

- Luau vector datatype support
- Luau readonly table attribute
- Other Luau improvements

## v0.8.0-beta.1

- Luau support
- Refactored ffi module. C glue is no longer required
- Added async support to `__index` and `__newindex` metamethods

## v0.7.4

- Improved `Lua::create_registry_value` to reuse previously expired registry keys.
  No need to call `Lua::expire_registry_values` when creating/dropping registry values.
- Added `Lua::replace_registry_value` to change value of an existing Registry Key
- Async calls optimization

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
- Added `chunk!` macro support to load chunks of Lua code using the Rust tokenizer and optionally capturing Rust variables.
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
- Provide safety guarantees for Lua state, which means that potentially unsafe operations, like loading C modules (using `require` or `package.loadlib`) are disabled. Equivalent to the previous `Lua::new()` function is `Lua::unsafe_new()`.
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
