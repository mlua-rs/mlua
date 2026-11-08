Implements the [`UserData`] trait for a Rust type.

This derive macro generates an implementation of [`UserData`] that exposes
struct fields to Lua and integrates with `#[mlua::userdata_impl]` for
registering methods.

Named fields are exposed as readable and writable fields in Lua by default.
Use `#[lua(...)]` on individual fields or methods to control how they are
registered.

```rust,ignore
use mlua::{Lua, Result, UserData};

#[derive(UserData)]
struct Rectangle {
    length: u32,
    width: u32,
}

#[mlua::userdata_impl]
impl Rectangle {
    #[lua(infallible)]
    fn new(length: u32, width: u32) -> Self {
        Self { length, width }
    }

    #[lua(getter, name = "area", infallible)]
    fn calculate_area(&self) -> u32 {
        self.length * self.width
    }

    fn diagonal(&self) -> Result<f64> {
        Ok(((self.length.pow(2) + self.width.pow(2)) as f64).sqrt())
    }
}
```

# Struct field attributes

Each named field can be annotated with `#[lua(...)]`:

| Attribute      | Description                                           |
| -------------- | ----------------------------------------------------- |
| `get`          | Expose a getter. The field becomes readable from Lua. |
| `set`          | Expose a setter. The field becomes writable from Lua. |
| `skip`         | Do not expose this field.                             |
| `name = "..."` | Override the Lua-facing name for the field.           |

If neither `get` nor `set` is specified, both are enabled.

Fields exposed as readable (via `get` or by default) must implement `Clone`.
The generated getter clones the field value when accessed from Lua.

# Methods registration

Use `#[mlua::userdata_impl]` on an `impl` block to register methods,
metamethods, and constants. All items in the block are registered automatically,
regardless of visibility.

## Method detection

The receiver type determines how a method is registered:

| Receiver    | Registration      |
| ----------- | ----------------- |
| `&self`     | `add_method`      |
| `&mut self` | `add_method_mut`  |
| `self`      | `add_method_once` |
| None        | `add_function`    |

A first parameter of type `&Lua` (or `&mlua::Lua`) is treated as the
Lua state reference and passed automatically.

## Method and constant attributes

Each item in the impl block can be annotated with `#[lua(...)]`:

| Attribute      | Applies to         | Description                                                                            |
| -------------- | ------------------ | -------------------------------------------------------------------------------------- |
| `skip`         | Methods, constants | Exclude this item from registration.                                                   |
| `name = "..."` | Methods, constants | Override the Lua-facing name.                                                          |
| `infallible`   | Methods            | Wrap the return value in `Ok(...)`.                                                    |
| `getter`       | Methods            | Register as a field getter. Must take `&self` and no Lua-facing arguments.             |
| `setter`       | Methods            | Register as a field setter. Must take `&[mut] self` and one value argument.            |
| `field`        | Methods, constants | Register as a static field. Methods must take no receiver and no Lua-facing arguments. |
| `meta`         | Methods, constants | Register as a metamethod. May be combined with `field` for meta static fields.         |

At most one of `getter`, `setter`, `field` may be specified on a method.

## Constants

Constants in an `#[mlua::userdata_impl]` block are registered as static
fields:

```rust,ignore
#[mlua::userdata_impl]
impl MyType {
    const VERSION: &str = "1.0";
    const COUNT: u32 = 42;
}
```

Use `#[lua(meta)]` on a constant to register it as a meta static field.

## Metamethods

Annotate a method with `#[lua(meta)]` to register it as a Lua metamethod.
The metamethod name is inferred from the function name when it starts with
`__`. Use `name = "..."` to specify the name explicitly.

```rust,ignore
#[mlua::userdata_impl]
impl MyType {
    #[lua(meta, infallible)]
    fn __add(&self, other: &Self) -> Self { ... }

    #[lua(meta, name = "__call", infallible)]
    fn construct(this: mlua::AnyUserData, value: u32) -> Self { ... }
}
```

A metamethod with a `self` receiver is registered via `add_meta_method`
and behaves like a regular method. A metamethod without a receiver is
registered via `add_meta_function` and receives exactly the values Lua passes.
Declare every argument Lua provides, in order:

- Binary metamethods (`__add`, `__sub`, `__eq`, `__concat`, ...) receive both
  operands, so both must be declared. This is also the way to support reversed
  operands (e.g. `2 + obj`), where the userdata is the second argument.
- Method-style metamethods (`__call`, `__index`, `__newindex`, ...) receive the
  object as their first argument. When registered without a `self` receiver
  (for example a constructor invoked as `MyType(value)`), declare that leading
  argument explicitly (typically `mlua::AnyUserData`), even if it is ignored.

```rust,ignore
#[mlua::userdata_impl]
impl Vec2 {
    // No receiver: both operands are declared and passed directly by Lua.
    #[lua(meta, infallible, name = "__add")]
    fn add(a: &Vec2, b: &Vec2) -> Vec2 { ... }
}
```

## Reference parameters

Reference parameters in method signatures are automatically mapped to
the appropriate callback wrapper types:

| Parameter type | Callback type       |
| -------------- | ------------------- |
| `&str`         | `BorrowedStr`       |
| `&[u8]`        | `BorrowedBytes`     |
| `&T`           | `UserDataRef<T>`    |
| `&mut T`       | `UserDataRefMut<T>` |

## Async methods

Async methods are supported and registered via the corresponding async
variants (`add_async_method`, `add_async_method_mut`, etc.).

# Limitations

Generics are not supported. Wrap a generic type in a concrete newtype
instead.

Union types cannot derive `UserData`.

Enum types are accepted but generate no field registrations. All method
registration must be done via `#[mlua::userdata_impl]`.

[`UserData`]: crate::UserData
