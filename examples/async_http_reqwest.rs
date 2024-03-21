use mlua::{chunk, ExternalResult, Lua, LuaSerdeExt, Result, Value};

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    let lua = Lua::new();

    let fetch_json = lua.create_async_function(|lua, uri: String| async move {
        let resp = reqwest::get(&uri)
            .await
            .and_then(|resp| resp.error_for_status())
            .into_lua_err()?;
        let json = resp.json::<serde_json::Value>().await.into_lua_err()?;
        lua.to_value(&json)
    })?;

    let dbg = lua.create_function(|_, value: Value| {
        println!("{value:#?}");
        Ok(())
    })?;

    let f = lua
        .load(chunk! {
            local res = $fetch_json(...)
            $dbg(res)
        })
        .into_function()?;

    f.call_async("https://httpbin.org/anything?arg0=val0").await
}
