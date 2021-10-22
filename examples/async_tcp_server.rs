use std::io;
use std::net::SocketAddr;
use std::rc::Rc;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::task;

use mlua::{
    chunk, AnyUserData, Function, Lua, RegistryKey, String as LuaString, UserData, UserDataMethods,
};

struct LuaTcpStream(TcpStream);

impl UserData for LuaTcpStream {
    fn add_methods<'lua, M: UserDataMethods<'lua, Self>>(methods: &mut M) {
        methods.add_method("peer_addr", |_, this, ()| {
            Ok(this.0.peer_addr()?.to_string())
        });

        methods.add_async_function(
            "read",
            |lua, (this, size): (AnyUserData, usize)| async move {
                let mut this = this.borrow_mut::<Self>()?;
                let mut buf = vec![0; size];
                let n = this.0.read(&mut buf).await?;
                buf.truncate(n);
                lua.create_string(&buf)
            },
        );

        methods.add_async_function(
            "write",
            |_, (this, data): (AnyUserData, LuaString)| async move {
                let mut this = this.borrow_mut::<Self>()?;
                let n = this.0.write(&data.as_bytes()).await?;
                Ok(n)
            },
        );

        methods.add_async_function("close", |_, this: AnyUserData| async move {
            let mut this = this.borrow_mut::<Self>()?;
            this.0.shutdown().await?;
            Ok(())
        });
    }
}

async fn run_server(lua: Lua, handler: RegistryKey) -> io::Result<()> {
    let addr: SocketAddr = ([127, 0, 0, 1], 3000).into();
    let listener = TcpListener::bind(addr).await.expect("cannot bind addr");

    println!("Listening on {}", addr);

    let lua = Rc::new(lua);
    let handler = Rc::new(handler);
    loop {
        let (stream, _) = match listener.accept().await {
            Ok(res) => res,
            Err(err) if is_transient_error(&err) => continue,
            Err(err) => return Err(err),
        };

        let lua = lua.clone();
        let handler = handler.clone();
        task::spawn_local(async move {
            let handler: Function = lua
                .registry_value(&handler)
                .expect("cannot get Lua handler");

            let stream = LuaTcpStream(stream);
            if let Err(err) = handler.call_async::<_, ()>(stream).await {
                eprintln!("{}", err);
            }
        });
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let lua = Lua::new();

    // Create Lua handler function
    let handler_fn = lua
        .load(chunk! {
            function(stream)
                local peer_addr = stream:peer_addr()
                print("connected from "..peer_addr)

                while true do
                    local data = stream:read(100)
                    data = data:match("^%s*(.-)%s*$") // trim
                    print("["..peer_addr.."] "..data)
                    if data == "bye" then
                        stream:write("bye bye\n")
                        stream:close()
                        return
                    end
                    stream:write("echo: "..data.."\n")
                end
            end
        })
        .eval::<Function>()
        .expect("cannot create Lua handler");

    // Store it in the Registry
    let handler = lua
        .create_registry_value(handler_fn)
        .expect("cannot store Lua handler");

    task::LocalSet::new()
        .run_until(run_server(lua, handler))
        .await
        .expect("cannot run server")
}

fn is_transient_error(e: &io::Error) -> bool {
    e.kind() == io::ErrorKind::ConnectionRefused
        || e.kind() == io::ErrorKind::ConnectionAborted
        || e.kind() == io::ErrorKind::ConnectionReset
}
