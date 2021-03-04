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
