#[cfg(feature = "luau")]
use mlua::Lua;

#[test]
fn test_luau_continuation() {
    let lua = Lua::new();

    let cont_func = lua.create_function_with_luau_continuation(
        |lua, a: u64| Ok(a + 1),
        |lua, status, a: u64| {
            println!("Reached cont");
            Ok(a + 2)
        }
    ).expect("Failed to create cont_func");

    // Ensure normal calls work still
    assert_eq!(
        lua.load("local cont_func = ...\nreturn cont_func(1)")
        .call::<u64>(cont_func).expect("Failed to call cont_func"),
        2
    );

    let always_yield = lua.create_function(
        |lua, ()| {
            unsafe { lua.set_yield_args((42, "69420".to_string(), 45.6))? }
            Ok(())
        })
        .unwrap();

    let thread = lua.create_thread(always_yield).unwrap();
    assert_eq!(thread.resume::<(i32, String, f32)>(()).unwrap(), (42, String::from("69420"), 45.6));

    // Trigger the continuation
    let cont_func = lua.create_function_with_luau_continuation(
        |lua, a: u64| {
            unsafe { 
                match lua.set_yield_args(a) {
                    Ok(()) => println!("set_yield_args called"),
                    Err(e) => println!("{:?}", e)
                } 
            }
            Ok(())
        },
        |lua, status, a: u64| {
            println!("Reached cont");
            Ok(a + 39)
        }
    ).expect("Failed to create cont_func");

    let luau_func = lua.load("
        local cont_func = ...
        local res = cont_func(1)
        return res + 1
    ").into_function().expect("Failed to create function");
    let th = lua.create_thread(luau_func).expect("Failed to create luau thread");

    let v = th.resume::<mlua::MultiValue>(cont_func).expect("Failed to resume");
    let v = th.resume::<i32>(v).expect("Failed to load continuation");

    assert_eq!(v, 41);

    let always_yield = lua.create_function_with_luau_continuation(
        |lua, ()| {
            unsafe { lua.set_yield_args((42, "69420".to_string(), 45.6))? }
            Ok(())
        },
        |lua, _, mv: mlua::MultiValue| {
            println!("Reached second continuation");
            if mv.is_empty() {
                return Ok(mv);
            }
            Err(mlua::Error::external(format!("a{}", mv.len())))
        }
    )
    .unwrap();

    let thread = lua.create_thread(always_yield).unwrap();
    let mv = thread.resume::<mlua::MultiValue>(()).unwrap();
    assert!(thread.resume::<String>(mv).unwrap_err().to_string().starts_with("a3"));
}