use mlua::{AnyUserData, Lua, Table, UserData, Result};

fn main() -> Result<()> {
    let lua = Lua::new();
    let globals = lua.globals();

    // Should not allow userdata borrow to outlive lifetime of AnyUserData handle
    struct MyUserData;
    impl UserData for MyUserData {};
    let _userdata_ref;
    {
        let touter = globals.get::<_, Table>("touter")?;
        touter.set("userdata", lua.create_userdata(MyUserData)?)?;
        let userdata = touter.get::<_, AnyUserData>("userdata")?;
        _userdata_ref = userdata.borrow::<MyUserData>();
        //~^ error: `userdata` does not live long enough
    }
    Ok(())
}
