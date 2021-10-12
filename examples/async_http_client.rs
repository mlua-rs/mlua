use std::collections::HashMap;

use hyper::body::{Body as HyperBody, HttpBody as _};
use hyper::Client as HyperClient;

use mlua::{chunk, AnyUserData, ExternalResult, Lua, Result, UserData, UserDataMethods};

struct BodyReader(HyperBody);

impl UserData for BodyReader {
    fn add_methods<'lua, M: UserDataMethods<'lua, Self>>(methods: &mut M) {
        methods.add_async_function("read", |lua, reader: AnyUserData| async move {
            let mut reader = reader.borrow_mut::<Self>()?;
            if let Some(bytes) = reader.0.data().await {
                let bytes = bytes.to_lua_err()?;
                return Some(lua.create_string(&bytes)).transpose();
            }
            Ok(None)
        });
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let lua = Lua::new();

    let fetch_url = lua.create_async_function(|lua, uri: String| async move {
        let client = HyperClient::new();
        let uri = uri.parse().to_lua_err()?;
        let resp = client.get(uri).await.to_lua_err()?;

        let lua_resp = lua.create_table()?;
        lua_resp.set("status", resp.status().as_u16())?;

        let mut headers = HashMap::new();
        for (key, value) in resp.headers() {
            headers
                .entry(key.as_str())
                .or_insert(Vec::new())
                .push(value.to_str().to_lua_err()?);
        }

        lua_resp.set("headers", headers)?;
        lua_resp.set("body", BodyReader(resp.into_body()))?;

        Ok(lua_resp)
    })?;

    let f = lua
        .load(chunk! {
            local res = $fetch_url(...)
            print("status: "..res.status)
            for key, vals in pairs(res.headers) do
                for _, val in ipairs(vals) do
                    print(key..": "..val)
                end
            end
            repeat
                local body = res.body:read()
                if body then
                    print(body)
                end
            until not body
        })
        .into_function()?;

    f.call_async("http://httpbin.org/ip").await
}
