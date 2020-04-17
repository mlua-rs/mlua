use std::net::Shutdown;
use std::rc::Rc;

use bstr::BString;
use tokio::net::{TcpListener, TcpStream};
use tokio::prelude::*;
use tokio::sync::Mutex;
use tokio::task;

use mlua::{Function, Lua, Result, Thread, UserData, UserDataMethods};

#[derive(Clone)]
struct LuaTcpListener(Option<Rc<Mutex<TcpListener>>>);

#[derive(Clone)]
struct LuaTcpStream(Rc<Mutex<TcpStream>>);

impl UserData for LuaTcpListener {
    fn add_methods<'lua, M: UserDataMethods<'lua, Self>>(methods: &mut M) {
        methods.add_async_function("bind", |_, addr: String| async {
            let listener = TcpListener::bind(addr).await?;
            Ok(LuaTcpListener(Some(Rc::new(Mutex::new(listener)))))
        });

        methods.add_async_method("accept", |_, listener, ()| async {
            let (stream, _) = listener.0.unwrap().lock().await.accept().await?;
            Ok(LuaTcpStream(Rc::new(Mutex::new(stream))))
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
            let mut stream = stream.0.lock().await;
            let n = stream.read(&mut buf).await?;
            buf.truncate(n);
            Ok(BString::from(buf))
        });

        methods.add_async_method("write", |_, stream, data: BString| async move {
            let mut stream = stream.0.lock().await;
            let n = stream.write(&data).await?;
            Ok(n)
        });

        methods.add_async_method("close", |_, stream, ()| async move {
            stream.0.lock().await.shutdown(Shutdown::Both)?;
            Ok(())
        });
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let lua = Lua::new();

    let globals = lua.globals();
    globals.set("tcp", LuaTcpListener(None))?;

    globals.set(
        "spawn",
        lua.create_function(move |lua: &Lua, func: Function| {
            let fut = lua.create_thread(func)?.into_async::<_, ()>(());
            task::spawn_local(async move { fut.await.unwrap() });
            Ok(())
        })?,
    )?;

    let thread = lua
        .load(
            r#"
            coroutine.create(function ()
                local listener = tcp.bind("0.0.0.0:1234")
                print("listening on 0.0.0.0:1234")
                while true do
                    local stream = listener:accept()
                    print("connected from " .. stream:peer_addr())
                    spawn(function()
                        while true do
                            local data = stream:read(100)
                            data = data:match("^%s*(.-)%s*$") -- trim
                            print(data)
                            stream:write("got: "..data.."\n")
                            if data == "exit" then
                                stream:close()
                                break
                            end
                        end
                    end)
                end
            end)
        "#,
        )
        .eval::<Thread>()?;

    thread.into_async(()).await
}
