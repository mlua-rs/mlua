use bytes::Bytes;
use http_body_util::BodyExt;
use http_body_util::Empty;
use hyper::body::Incoming;
use hyper_util::client::legacy::Client as HyperClient;
use std::collections::HashMap;

use mlua::{chunk, ExternalResult, Lua, Result, UserData, UserDataMethods};

struct BodyReader(Incoming);

impl UserData for BodyReader {
    fn add_methods<'lua, M: UserDataMethods<'lua, Self>>(methods: &mut M) {
        methods.add_async_method_mut("read", |lua, reader, ()| async move {
            let mut summarize = Vec::new(); // Create a vector to accumulate the bytes

            loop {
                match reader.0.frame().await {
                    Some(Ok(bytes)) => {
                        if let Ok(data) = bytes.into_data() {
                            summarize.extend(data); // Append the bytes to the summarize variable
                        }
                    }
                    Some(Err(_)) => break, // Break on error
                    None => break,         // Break if no more frames
                }
            }

            if !summarize.is_empty() {
                // If summarize has collected data, return it as a Lua string
                Ok(Some(lua.create_string(&summarize)?))
            } else {
                // Return None if no data was collected
                Ok(None)
            }
        });
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    let lua = Lua::new();

    let fetch_url = lua.create_async_function(|lua, uri: String| async move {
        let client =
            HyperClient::builder(hyper_util::rt::TokioExecutor::new()).build_http::<Empty<Bytes>>();
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
            local body = res.body:read()
            if body then
            print(body)
            end
            until not body
        })
        .into_function()?;

    f.call_async("http://httpbin.org/ip").await
}
