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

## Building a C module on macOS fails to link with `Undefined symbols` errors. How to fix?

This is different from the case above. When you build a standalone C module (a crate with
`crate-type = ["cdylib"]` and the `module` feature), the `lua_xxx` symbols are meant to stay
*undefined* at link time and be resolved at load time from the host interpreter (e.g. `lua5.4`).

On Linux this works out of the box. macOS, however, rejects undefined symbols in a dylib by
default, so you must pass `-undefined dynamic_lookup` to the linker. Add a `build.rs` to your
module crate:

```rust
fn main() {
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if target_os == "macos" {
        println!("cargo:rustc-cdylib-link-arg=-undefined");
        println!("cargo:rustc-cdylib-link-arg=dynamic_lookup");
    }
}
```

Check `CARGO_CFG_TARGET_OS` rather than `cfg!(target_os = ...)` here: a build script is itself
compiled for the machine that *runs* the build, so `cfg!(target_os)` reports that machine and gives
the wrong answer when cross-compiling. `CARGO_CFG_TARGET_OS` is the platform you are building for.

These flags must be set in *your module crate*, not in `mlua` or `mlua-sys`: build-script link
arguments only apply to a cdylib built by the same package, so they cannot be injected by a
dependency.

## I want to add support for a Lua VM fork to mlua. Do you accept pull requests?

Adding new feature flag to support a Lua VM fork is a major step that requires huge effort to maintain it.
Regular updates, testing, checking compatibility, etc.
That's why I don't plan to support new Lua VM forks or other languages in mlua.
