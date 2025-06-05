use mlua::IntoLuaMulti;
#[cfg(feature = "luau")]
use mlua::Lua;

#[test]
fn test_luau_continuation() {
    // Yielding continuation
    mlua::Lua::set_fflag("LuauYieldableContinuations", true).unwrap();

    let lua = Lua::new();

    let cont_func = lua
        .create_function_with_luau_continuation(
            |_lua, a: u64| Ok(a + 1),
            |_lua, _status, a: u64| {
                println!("Reached cont");
                Ok(a + 2)
            },
        )
        .expect("Failed to create cont_func");

    // Ensure normal calls work still
    assert_eq!(
        lua.load("local cont_func = ...\nreturn cont_func(1)")
            .call::<u64>(cont_func)
            .expect("Failed to call cont_func"),
        2
    );

    // basic yield test before we go any further
    let always_yield = lua
        .create_function(|lua, ()| {
            lua.set_yield_args((42, "69420".to_string(), 45.6))?;
            Ok(())
        })
        .unwrap();

    let thread = lua.create_thread(always_yield).unwrap();
    assert_eq!(
        thread.resume::<(i32, String, f32)>(()).unwrap(),
        (42, String::from("69420"), 45.6)
    );

    // Trigger the continuation
    let cont_func = lua
        .create_function_with_luau_continuation(
            |lua, a: u64| {
                match lua.set_yield_args(a) {
                    Ok(()) => println!("set_yield_args called"),
                    Err(e) => println!("{:?}", e),
                };
                Ok(())
            },
            |_lua, _status, a: u64| {
                println!("Reached cont");
                Ok(a + 39)
            },
        )
        .expect("Failed to create cont_func");

    let luau_func = lua
        .load(
            "
        local cont_func = ...
        local res = cont_func(1)
        return res + 1
    ",
        )
        .into_function()
        .expect("Failed to create function");

    let th = lua
        .create_thread(luau_func)
        .expect("Failed to create luau thread");

    let v = th
        .resume::<mlua::MultiValue>(cont_func)
        .expect("Failed to resume");
    let v = th.resume::<i32>(v).expect("Failed to load continuation");

    assert_eq!(v, 41);

    let always_yield = lua
        .create_function_with_luau_continuation(
            |lua, ()| {
                lua.set_yield_args((42, "69420".to_string(), 45.6))?;
                Ok(())
            },
            |_lua, _, mv: mlua::MultiValue| {
                println!("Reached second continuation");
                if mv.is_empty() {
                    return Ok(mv);
                }
                Err(mlua::Error::external(format!("a{}", mv.len())))
            },
        )
        .unwrap();

    let thread = lua.create_thread(always_yield).unwrap();
    let mv = thread.resume::<mlua::MultiValue>(()).unwrap();
    assert!(thread
        .resume::<String>(mv)
        .unwrap_err()
        .to_string()
        .starts_with("a3"));

    let cont_func = lua
        .create_function_with_luau_continuation(
            |lua, a: u64| {
                match lua.set_yield_args((a + 1, 1)) {
                    Ok(()) => println!("set_yield_args called"),
                    Err(e) => println!("{:?}", e),
                }
                Ok(())
            },
            |lua, _status, args: mlua::MultiValue| {
                println!("Reached cont recursive: {:?}", args);

                if args.len() == 5 {
                    return 6_i32.into_lua_multi(lua);
                }

                lua.set_yield_args((args.len() + 1, args))?; // thread state becomes Integer(2), Integer(1), Integer(8), Integer(9), Integer(10)
                (1, 2, 3, 4, 5).into_lua_multi(lua) // this value is ignored
            },
        )
        .expect("Failed to create cont_func");

    let luau_func = lua
        .load(
            "
        local cont_func = ...
        local res = cont_func(1)
        return res + 1
    ",
        )
        .into_function()
        .expect("Failed to create function");
    let th = lua
        .create_thread(luau_func)
        .expect("Failed to create luau thread");

    let v = th
        .resume::<mlua::MultiValue>(cont_func)
        .expect("Failed to resume");
    println!("v={:?}", v);

    let v = th
        .resume::<mlua::MultiValue>(v)
        .expect("Failed to load continuation");
    let v = th
        .resume::<mlua::MultiValue>(v)
        .expect("Failed to load continuation");
    let v = th
        .resume::<mlua::MultiValue>(v)
        .expect("Failed to load continuation");

    // (2, 1) followed by ()
    assert_eq!(v.len(), 2 + 3);

    let v = th.resume::<i32>(v).expect("Failed to load continuation");

    assert_eq!(v, 7);
}
