use mlua::{chunk, ExternalResult, Lua, LuaSerdeExt, Result};

#[tokio::main]
async fn main() -> Result<()> {
    let lua = Lua::new();

    let null = lua.null();

    let fetch_json = lua.create_async_function(|lua, uri: String| async move {
        let resp = reqwest::get(&uri)
            .await
            .and_then(|resp| resp.error_for_status())
            .to_lua_err()?;
        let json = resp.json::<serde_json::Value>().await.to_lua_err()?;
        lua.to_value(&json)
    })?;

    let f = lua
        .load(chunk! {
            function print_r(t, indent)
                local indent = indent or ""
                for k, v in pairs(t) do
                    io.write(indent, tostring(k))
                    if type(v) == "table" then io.write(":\n") print_r(v, indent.."  ")
                    else io.write(": ", v == $null and "null" or tostring(v), "\n") end
                end
            end

            local res = $fetch_json(...)
            print_r(res)
        })
        .into_function()?;

    f.call_async("https://httpbin.org/anything?arg0=val0").await
}
