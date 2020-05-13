use std::net::SocketAddr;
use std::sync::Arc;

use hyper::server::conn::AddrStream;
use hyper::service::{make_service_fn, service_fn};
use hyper::{Body, Request, Response, Server};

use mlua::{Error, Function, Lua, Result, Table, UserData, UserDataMethods};

#[derive(Clone)]
struct LuaRequest(Arc<(SocketAddr, Request<Body>)>);

impl UserData for LuaRequest {
    fn add_methods<'lua, M: UserDataMethods<'lua, Self>>(methods: &mut M) {
        methods.add_method("remote_addr", |_lua, req, ()| Ok((req.0).0.to_string()));
        methods.add_method("method", |_lua, req, ()| Ok((req.0).1.method().to_string()));
    }
}

async fn run_server(handler: Function<'static>) -> Result<()> {
    let make_svc = make_service_fn(|socket: &AddrStream| {
        let remote_addr = socket.remote_addr();
        let handler = handler.clone();
        async move {
            Ok::<_, Error>(service_fn(move |req: Request<Body>| {
                let handler = handler.clone();
                async move {
                    let lua_req = LuaRequest(Arc::new((remote_addr, req)));
                    let lua_resp: Table = handler.call_async(lua_req).await?;
                    let body = lua_resp
                        .get::<_, Option<String>>("body")?
                        .unwrap_or_default();

                    let mut resp = Response::builder()
                        .status(lua_resp.get::<_, Option<u16>>("status")?.unwrap_or(200));

                    if let Some(headers) = lua_resp.get::<_, Option<Table>>("headers")? {
                        for pair in headers.pairs::<String, String>() {
                            let (h, v) = pair?;
                            resp = resp.header(&h, v);
                        }
                    }

                    Ok::<_, Error>(resp.body(Body::from(body)).unwrap())
                }
            }))
        }
    });

    let addr = ([127, 0, 0, 1], 3000).into();
    let server = Server::bind(&addr).executor(LocalExec).serve(make_svc);

    println!("Listening on http://{}", addr);

    tokio::task::LocalSet::new()
        .run_until(server)
        .await
        .map_err(Error::external)
}

#[tokio::main]
async fn main() -> Result<()> {
    let lua = Lua::new().into_static();

    let handler: Function = lua
        .load(
            r#"
        function(req)
            return {
                status = 200,
                headers = {
                    ["X-Req-Method"] = req:method(),
                    ["X-Remote-Addr"] = req:remote_addr(),
                },
                body = "Hello, World!"
            }
        end
    "#,
        )
        .eval()?;

    run_server(handler).await?;

    // Consume the static reference and drop it.
    // This is safe as long as we don't hold any other references to Lua
    // or alive resources.
    unsafe { Lua::from_static(lua) };
    Ok(())
}

#[derive(Clone, Copy, Debug)]
struct LocalExec;

impl<F> hyper::rt::Executor<F> for LocalExec
where
    F: std::future::Future + 'static, // not requiring `Send`
{
    fn execute(&self, fut: F) {
        tokio::task::spawn_local(fut);
    }
}
