use std::collections::HashMap;

use http_body_util::BodyExt as _;
use hyper::body::Incoming;
use hyper_util::client::legacy::Client as HyperClient;
use hyper_util::rt::TokioExecutor;

use mlua::{chunk, ExternalResult, Lua, Result, UserData, UserDataMethods};

struct BodyReader(Incoming);

impl UserData for BodyReader {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        // Every call returns a next chunk
        methods.add_async_method_mut("read", |lua, mut reader, ()| async move {
            if let Some(bytes) = reader.0.frame().await {
                if let Some(bytes) = bytes.into_lua_err()?.data_ref() {
                    return Some(lua.create_string(&bytes)).transpose();
                }
            }
            Ok(None)
        });
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    let lua = Lua::new();

    let fetch_url = lua.create_async_function(|lua, uri: String| async move {
        let client = HyperClient::builder(TokioExecutor::new()).build_http::<String>();
        let uri = uri.parse().into_lua_err()?;
        let resp = client.get(uri).await.into_lua_err()?;

        let lua_resp = lua.create_table()?;
        lua_resp.set("status", resp.status().as_u16())?;

        let mut headers = HashMap::new();
        for (key, value) in resp.headers() {
            headers
                .entry(key.as_str())
                .or_insert(Vec::new())
                .push(value.to_str().into_lua_err()?);
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
                local chunk = res.body:read()
                if chunk then
                    print(chunk)
                end
            until not chunk
        })
        .into_function()?;

    f.call_async("http://httpbin.org/ip").await
}
