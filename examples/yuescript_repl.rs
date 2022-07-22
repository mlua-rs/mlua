use mlua::{Error, Function, Lua, LuaOptions, MultiValue, Result, StdLib, Table, Value};
use rustyline::Editor;

fn get_yue(lua: &Lua) -> Result<Table> {
    Ok(lua
        .globals()
        .get::<_, Table>("package")?
        .get::<_, Table>("loaded")?
        .get::<_, Table>("yue")?)
}

fn yue_options(lua: &Lua, line_offset: i32) -> Result<Table> {
    let options = lua.create_table()?;
    options.set("lint_global", false)?;
    options.set("implicit_return_root", true)?;
    options.set("reserve_line_number", true)?;
    options.set("space_over_tab", false)?;
    options.set("same_module", true)?;
    options.set("line_offset", line_offset)?;
    Ok(options)
}

fn load_yue<'a>(lua: &'a Lua, source: &String) -> Result<Function<'a>> {
    let mut res = get_yue(lua)?
        .get::<_, Function>("loadstring")?
        .call::<_, MultiValue>((
            Value::String(lua.create_string(format!("global *\n{}", source).as_str())?),
            Value::String(lua.create_string("=(repl)")?),
            Value::Table(yue_options(lua, -1)?),
        ))?;

    match res.pop_front() {
        Some(Value::Function(chunk)) => Ok(chunk),
        _ => match res.pop_front() {
            Some(Value::String(message)) => Err(Error::SyntaxError {
                message: message.to_str()?.into(),
                incomplete_input: false,
            }),
            _ => Err(Error::SyntaxError {
                message: "Compilation failed".into(),
                incomplete_input: false,
            }),
        },
    }
}

fn main() -> Result<()> {
    let lua = Lua::new_with(StdLib::ALL_SAFE | StdLib::YUE, LuaOptions::new())?;
    let mut editor = Editor::<()>::new();

    loop {
        let mut line = String::new();
        let mut is_multi_line = false;

        loop {
            match editor.readline(if is_multi_line { ">> " } else { "> " }) {
                Ok(input) => match input.as_str() {
                    "$" if !is_multi_line => is_multi_line = true,
                    "" if is_multi_line => is_multi_line = false,
                    _ => {
                        line.push_str(&input);
                        if is_multi_line {
                            line.push('\n');
                        }
                    }
                },
                Err(_) => return Ok(()),
            }

            if is_multi_line {
                continue;
            }

            match load_yue(&lua, &line) {
                Ok(function) => {
                    editor.add_history_entry(line);

                    match function.call::<_, MultiValue>(()) {
                        Ok(values) => {
                            println!(
                                "{}",
                                values
                                    .iter()
                                    .map(|value| format!("{:?}", value))
                                    .collect::<Vec<_>>()
                                    .join("\t")
                            );
                        }
                        Err(e) => {
                            eprintln!("error: {}", e);
                            break;
                        }
                    }
                    break;
                }
                Err(e) => {
                    eprintln!("error: {}", e);
                    break;
                }
            }
        }
    }
}
