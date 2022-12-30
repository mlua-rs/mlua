use std::future::Future;
use std::net::SocketAddr;
use std::pin::Pin;
use std::rc::Rc;
use std::task::{Context, Poll};

use hyper::server::conn::AddrStream;
use hyper::service::Service;
use hyper::{Body, Request, Response, Server};

use mlua::{
    chunk, Error as LuaError, Function, Lua, String as LuaString, Table, UserData, UserDataMethods,
};

struct LuaRequest(SocketAddr, Request<Body>);

impl UserData for LuaRequest {
    fn add_methods<'lua, M: UserDataMethods<'lua, Self>>(methods: &mut M) {
        methods.add_method("remote_addr", |_lua, req, ()| Ok((req.0).to_string()));
        methods.add_method("method", |_lua, req, ()| Ok((req.1).method().to_string()));
    }
}

pub struct Svc(Rc<Lua>, SocketAddr);

impl Service<Request<Body>> for Svc {
    type Response = Response<Body>;
    type Error = LuaError;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>>>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: Request<Body>) -> Self::Future {
        // If handler returns an error then generate 5xx response
        let lua = self.0.clone();
        let lua_req = LuaRequest(self.1, req);
        Box::pin(async move {
            let handler: Function = lua.named_registry_value("http_handler")?;
            match handler.call_async::<_, Table>(lua_req).await {
                Ok(lua_resp) => {
                    let status = lua_resp.get::<_, Option<u16>>("status")?.unwrap_or(200);
                    let mut resp = Response::builder().status(status);

                    // Set headers
                    if let Some(headers) = lua_resp.get::<_, Option<Table>>("headers")? {
                        for pair in headers.pairs::<String, LuaString>() {
                            let (h, v) = pair?;
                            resp = resp.header(&h, v.as_bytes());
                        }
                    }

                    let body = lua_resp
                        .get::<_, Option<LuaString>>("body")?
                        .map(|b| Body::from(b.as_bytes().to_vec()))
                        .unwrap_or_else(Body::empty);

                    Ok(resp.body(body).unwrap())
                }
                Err(err) => {
                    eprintln!("{}", err);
                    Ok(Response::builder()
                        .status(500)
                        .body(Body::from("Internal Server Error"))
                        .unwrap())
                }
            }
        })
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let lua = Rc::new(Lua::new());

    // Create Lua handler function
    let handler: Function = lua
        .load(chunk! {
            function(req)
                return {
                    status = 200,
                    headers = {
                        ["X-Req-Method"] = req:method(),
                        ["X-Remote-Addr"] = req:remote_addr(),
                    },
                    body = "Hello from Lua!\n"
                }
            end
        })
        .eval()
        .expect("cannot create Lua handler");

    // Store it in the Registry
    lua.set_named_registry_value("http_handler", handler)
        .expect("cannot store Lua handler");

    let addr = ([127, 0, 0, 1], 3000).into();
    let server = Server::bind(&addr).executor(LocalExec).serve(MakeSvc(lua));

    println!("Listening on http://{}", addr);

    // Create `LocalSet` to spawn !Send futures
    let local = tokio::task::LocalSet::new();
    local.run_until(server).await.expect("cannot run server")
}

struct MakeSvc(Rc<Lua>);

impl Service<&AddrStream> for MakeSvc {
    type Response = Svc;
    type Error = hyper::Error;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>>>>;

    fn poll_ready(&mut self, _: &mut Context) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, stream: &AddrStream) -> Self::Future {
        let lua = self.0.clone();
        let remote_addr = stream.remote_addr();
        Box::pin(async move { Ok(Svc(lua, remote_addr)) })
    }
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
