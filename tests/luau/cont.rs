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

    // does not work yet
    /*let always_yield = lua.create_function(|lua, ()| {
        unsafe { lua.yield_args((42, "69420")) }
    }).unwrap();

    let thread = lua.create_thread(always_yield).unwrap();
    assert_eq!(thread.resume::<(i32, String)>(()).unwrap(), (42, String::from("69420")));*/

    // Trigger the continuation
    let cont_func = lua.create_function_with_luau_continuation(
        |lua, a: u64| {
            unsafe { 
                match lua.yield_args(a) {
                    Ok(()) => println!("yield_args called"),
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
        return res
    ").into_function().expect("Failed to create function");
    let th = lua.create_thread(luau_func).expect("Failed to create luau thread");

    let v = th.resume::<mlua::MultiValue>(cont_func).expect("Failed to resume");
    let v = th.resume::<i32>(v).expect("Failed to load continuation");

    assert_eq!(v, 40);
}