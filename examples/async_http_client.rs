use std::collections::HashMap;

use hyper::Client as HyperClient;

use mlua::{Lua, Result, Thread, Error};

#[tokio::main]
async fn main() -> Result<()> {
    let lua = Lua::new();

    let fetch_url = lua.create_async_function(|lua, uri: String| async move {
        let client = HyperClient::new();
        let uri = uri.parse().map_err(Error::external)?;
        let resp = client.get(uri).await.map_err(Error::external)?;

        let lua_resp = lua.create_table()?;
        lua_resp.set("status", resp.status().as_u16())?;

        let mut headers = HashMap::new();
        for (key, value) in resp.headers().iter() {
            headers.entry(key.as_str()).or_insert(Vec::new()).push(value.to_str().unwrap());
        }
        lua_resp.set("headers", headers)?;

        let buf = hyper::body::to_bytes(resp).await.map_err(Error::external)?;
        lua_resp.set("body", String::from_utf8_lossy(&buf).into_owned())?;

        Ok(lua_resp)
    })?;

    let globals = lua.globals();
    globals.set("fetch_url", fetch_url)?;

    let thread = lua
        .load(
            r#"
            coroutine.create(function ()
                local res = fetch_url("http://httpbin.org/ip");
                print(res.status)
                for key, vals in pairs(res.headers) do
                    for _, val in ipairs(vals) do
                        print(key..": "..val)
                    end
                end
                print(res.body)
            end)
        "#,
        )
        .eval::<Thread>()?;

    thread.into_async(()).await
}
