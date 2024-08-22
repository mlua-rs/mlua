use std::io;
use std::net::SocketAddr;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use mlua::{chunk, BString, Function, Lua, UserData, UserDataMethods};

struct LuaTcpStream(TcpStream);

impl UserData for LuaTcpStream {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("peer_addr", |_, this, ()| Ok(this.0.peer_addr()?.to_string()));

        methods.add_async_method_mut("read", |lua, mut this, size| async move {
            let mut buf = vec![0; size];
            let n = this.0.read(&mut buf).await?;
            buf.truncate(n);
            lua.create_string(&buf)
        });

        methods.add_async_method_mut("write", |_, mut this, data: BString| async move {
            let n = this.0.write(&data).await?;
            Ok(n)
        });

        methods.add_async_method_mut("close", |_, mut this, ()| async move {
            this.0.shutdown().await?;
            Ok(())
        });
    }
}

async fn run_server(handler: Function) -> io::Result<()> {
    let addr: SocketAddr = ([127, 0, 0, 1], 3000).into();
    let listener = TcpListener::bind(addr).await.expect("cannot bind addr");

    println!("Listening on {}", addr);

    loop {
        let (stream, _) = match listener.accept().await {
            Ok(res) => res,
            Err(err) if is_transient_error(&err) => continue,
            Err(err) => return Err(err),
        };

        let handler = handler.clone();
        tokio::task::spawn(async move {
            let stream = LuaTcpStream(stream);
            if let Err(err) = handler.call_async::<()>(stream).await {
                eprintln!("{}", err);
            }
        });
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let lua = Lua::new();

    // Create Lua handler function
    let handler = lua
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

    run_server(handler).await.expect("cannot run server")
}

fn is_transient_error(e: &io::Error) -> bool {
    e.kind() == io::ErrorKind::ConnectionRefused
        || e.kind() == io::ErrorKind::ConnectionAborted
        || e.kind() == io::ErrorKind::ConnectionReset
}
