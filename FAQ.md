# mlua FAQ

This file is for general questions that don't fit into the README or crate docs.

## Loading a C module fails with error `undefined symbol: lua_xxx`. How to fix?

Add the following rustflags to your [.cargo/config](http://doc.crates.io/config.html) in order to properly export Lua symbols:

```toml
[target.x86_64-unknown-linux-gnu]
rustflags = ["-C", "link-args=-rdynamic"]

[target.x86_64-apple-darwin]
rustflags = ["-C", "link-args=-rdynamic"]
```

## I want to add support for a Lua VM fork to mlua. Do you accept pull requests?

Adding new feature flag to support a Lua VM fork is a major step that requires huge effort to maintain it.
Regular updates, testing, checking compatibility, etc.
That's why I don't plan to support new Lua VM forks or other languages in mlua.
