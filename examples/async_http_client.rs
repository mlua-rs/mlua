use std::collections::HashMap;
use std::sync::Arc;

use bstr::BString;
use hyper::body::{Body as HyperBody, HttpBody as _};
use hyper::Client as HyperClient;
use tokio::sync::Mutex;

use mlua::{Error, Lua, Result, UserData, UserDataMethods};

#[derive(Clone)]
struct BodyReader(Arc<Mutex<HyperBody>>);

impl BodyReader {
    fn new(body: HyperBody) -> Self {
        BodyReader(Arc::new(Mutex::new(body)))
    }
}

impl UserData for BodyReader {
    fn add_methods<'lua, M: UserDataMethods<'lua, Self>>(methods: &mut M) {
        methods.add_async_method("read", |_, reader, ()| async move {
            let mut reader = reader.0.lock().await;
            if let Some(bytes) = reader.data().await {
                let bytes = bytes.map_err(Error::external)?;
                return Ok(Some(BString::from(bytes.as_ref())));
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
        let uri = uri.parse().map_err(Error::external)?;
        let resp = client.get(uri).await.map_err(Error::external)?;

        let lua_resp = lua.create_table()?;
        lua_resp.set("status", resp.status().as_u16())?;

        let mut headers = HashMap::new();
        for (key, value) in resp.headers().iter() {
            headers
                .entry(key.as_str())
                .or_insert(Vec::new())
                .push(value.to_str().unwrap());
        }

        lua_resp.set("headers", headers)?;
        lua_resp.set("body", BodyReader::new(resp.into_body()))?;

        Ok(lua_resp)
    })?;

    let globals = lua.globals();
    globals.set("fetch_url", fetch_url)?;

    let f = lua
        .load(
            r#"
            local res = fetch_url(...)
            print(res.status)
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
        "#,
        )
        .into_function()?;

    f.call_async("http://httpbin.org/ip").await
}
