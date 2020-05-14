## v0.4.0-beta.1

- Lua 5.4 support with `MetaMethod::Close`.
- Provide safety guaraness for Lua states which means that potenially unsafe operations like loading C modules (using `require` or `package.loadlib`) are disabled. Equalient for the previous `Lua::new()` function is `Lua::unsafe_new()`.
- New `send` feature to require `Send`.
- Don't allow `'callback` outlive `'lua` in `Lua::create_function()`. This fixes [the unsoundness](tests/compile/static_callback_args.rs).
- Added `Lua::into_static()` to make `'static` Lua state. This is useful to spawn async Lua threads that requires `'static`.
- New function `Lua::set_memory_limit()` (similar to `rlua`) to enable memory restrictions in Lua VM (requires Lua >= 5.2).
- `Scope`, temporary removed in v0.3.0, is back with async support.
- Removed deprecated `Table::call()`
