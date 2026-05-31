Create a type that implements [`AsChunk`] and can capture Rust variables.

This macro allows to write Lua code directly in Rust code.

Rust variables can be referenced from Lua using `$` prefix, as shown in the example below.
User's Rust types needs to implement [`UserData`] or [`IntoLua`] traits.

Captured variables are **moved** into the chunk.

```rust
use mlua::{Lua, Result, chunk};

fn main() -> Result<()> {
    let lua = Lua::new();
    let name = "Rustacean";
    lua.load(chunk! {
        print("hello, " .. $name)
    }).exec()
}
```

## Syntax issues

Since the Rust tokenizer will tokenize Lua code, this imposes some restrictions.
The main thing to remember is:

- Use double quoted strings (`""`) instead of single quoted strings (`''`).

  (Single quoted strings only work if they contain a single character, since in Rust,
  `'a'` is a character literal).

- Using Lua comments `--` is not desirable in **stable** Rust and can have bad side effects.

  This is because procedural macros have Line/Column information available only in
  **nightly** Rust. Instead, Lua chunks represented as a big single line of code in stable Rust.

  As workaround, Rust comments `//` can be used.

Other minor limitations:

- Certain escape codes in string literals don't work. (Specifically: `\a`, `\b`, `\f`, `\v`,
  `\123` (octal escape codes), `\u`, and `\U`).

  These are accepted: : `\\`, `\n`, `\t`, `\r`, `\xAB` (hex escape codes), and `\0`.

- The `//` (floor division) operator is unusable, as its start a comment.

Everything else should work.

[`AsChunk`]: crate::chunk::AsChunk
[`UserData`]: crate::UserData
[`IntoLua`]: crate::IntoLua
