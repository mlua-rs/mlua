use std::sync::Arc;

use bstr::BString;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Mutex;
use tokio::task;

use mlua::{Function, Lua, Result, UserData, UserDataMethods};

#[derive(Clone)]
struct LuaTcp;

#[derive(Clone)]
struct LuaTcpListener(Arc<Mutex<TcpListener>>);

#[derive(Clone)]
struct LuaTcpStream(Arc<Mutex<TcpStream>>);

impl UserData for LuaTcp {
    fn add_methods<'lua, M: UserDataMethods<'lua, Self>>(methods: &mut M) {
        methods.add_async_function("bind", |_, addr: String| async move {
            let listener = TcpListener::bind(addr).await?;
            Ok(LuaTcpListener(Arc::new(Mutex::new(listener))))
        });
    }
}

impl UserData for LuaTcpListener {
    fn add_methods<'lua, M: UserDataMethods<'lua, Self>>(methods: &mut M) {
        methods.add_async_method("accept", |_, listener, ()| async move {
            let (stream, _) = listener.0.lock().await.accept().await?;
            Ok(LuaTcpStream(Arc::new(Mutex::new(stream))))
        });
    }
}

impl UserData for LuaTcpStream {
    fn add_methods<'lua, M: UserDataMethods<'lua, Self>>(methods: &mut M) {
        methods.add_async_method("peer_addr", |_, stream, ()| async move {
            Ok(stream.0.lock().await.peer_addr()?.to_string())
        });

        methods.add_async_method("read", |_, stream, size: usize| async move {
            let mut buf = vec![0; size];
            let n = stream.0.lock().await.read(&mut buf).await?;
            buf.truncate(n);
            Ok(BString::from(buf))
        });

        methods.add_async_method("write", |_, stream, data: BString| async move {
            let n = stream.0.lock().await.write(&data).await?;
            Ok(n)
        });

        methods.add_async_method("close", |_, stream, ()| async move {
            stream.0.lock().await.shutdown().await?;
            Ok(())
        });
    }
}

async fn run_server(lua: &'static Lua) -> Result<()> {
    let spawn = lua.create_function(move |_, func: Function| {
        task::spawn_local(async move { func.call_async::<_, ()>(()).await });
        Ok(())
    })?;

    let globals = lua.globals();
    globals.set("tcp", LuaTcp)?;
    globals.set("spawn", spawn)?;

    let server = lua
        .load(
            r#"
            local addr = ...
            local listener = tcp.bind(addr)
            print("listening on "..addr)

            local accept_new = true
            while true do
                local stream = listener:accept()
                local peer_addr = stream:peer_addr()
                print("connected from "..peer_addr)

                if not accept_new then
                    return
                end

                spawn(function()
                    while true do
                        local data = stream:read(100)
                        data = data:match("^%s*(.-)%s*$") -- trim
                        print("["..peer_addr.."] "..data)
                        if data == "bye" then
                            stream:write("bye bye\n")
                            stream:close()
                            return
                        end
                        if data == "exit" then
                            stream:close()
                            accept_new = false
                            return
                        end
                        stream:write("echo: "..data.."\n")
                    end
                end)
            end
        "#,
        )
        .into_function()?;

    task::LocalSet::new()
        .run_until(server.call_async::<_, ()>("0.0.0.0:1234"))
        .await
}

#[tokio::main]
async fn main() {
    let lua = Lua::new().into_static();

    run_server(lua).await.unwrap();

    // Consume the static reference and drop it.
    // This is safe as long as we don't hold any other references to Lua
    // or alive resources.
    unsafe { Lua::from_static(lua) };
}
