use std::convert::Infallible;
use std::future::Future;
use std::net::SocketAddr;
use std::rc::Rc;

use futures::future::LocalBoxFuture;
use http_body_util::{combinators::BoxBody, BodyExt as _, Empty, Full};
use hyper::body::{Bytes, Incoming};
use hyper::{Request, Response};
use hyper_util::rt::TokioIo;
use hyper_util::server::conn::auto::Builder as ServerConnBuilder;
use tokio::net::TcpListener;
use tokio::task::LocalSet;

use mlua::{
    chunk, Error as LuaError, Function, Lua, RegistryKey, String as LuaString, Table, UserData,
    UserDataMethods,
};

/// Wrapper around incoming request that implements UserData
struct LuaRequest(SocketAddr, Request<Incoming>);

impl UserData for LuaRequest {
    fn add_methods<'lua, M: UserDataMethods<'lua, Self>>(methods: &mut M) {
        methods.add_method("remote_addr", |_, req, ()| Ok((req.0).to_string()));
        methods.add_method("method", |_, req, ()| Ok((req.1).method().to_string()));
        methods.add_method("path", |_, req, ()| Ok(req.1.uri().path().to_string()));
    }
}

/// Service that handles incoming requests
#[derive(Clone)]
pub struct Svc {
    lua: Rc<Lua>,
    handler: Rc<RegistryKey>,
    peer_addr: SocketAddr,
}

impl Svc {
    pub fn new(lua: Rc<Lua>, handler: Rc<RegistryKey>, peer_addr: SocketAddr) -> Self {
        Self {
            lua,
            handler,
            peer_addr,
        }
    }
}

impl hyper::service::Service<Request<Incoming>> for Svc {
    type Response = Response<BoxBody<Bytes, Infallible>>;
    type Error = LuaError;
    type Future = LocalBoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn call(&self, req: Request<Incoming>) -> Self::Future {
        // If handler returns an error then generate 5xx response
        let lua = self.lua.clone();
        let handler_key = self.handler.clone();
        let lua_req = LuaRequest(self.peer_addr, req);
        Box::pin(async move {
            let handler: Function = lua.registry_value(&handler_key)?;
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

                    // Set body
                    let body = lua_resp
                        .get::<_, Option<LuaString>>("body")?
                        .map(|b| Full::new(Bytes::copy_from_slice(b.as_bytes())).boxed())
                        .unwrap_or_else(|| Empty::<Bytes>::new().boxed());

                    Ok(resp.body(body).unwrap())
                }
                Err(err) => {
                    eprintln!("{}", err);
                    Ok(Response::builder()
                        .status(500)
                        .body(Full::new(Bytes::from("Internal Server Error")).boxed())
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
    let handler: RegistryKey = lua
        .load(chunk! {
            function(req)
                return {
                    status = 200,
                    headers = {
                        ["X-Req-Method"] = req:method(),
                        ["X-Req-Path"] = req:path(),
                        ["X-Remote-Addr"] = req:remote_addr(),
                    },
                    body = "Hello from Lua!\n"
                }
            end
        })
        .eval()
        .expect("Failed to create Lua handler");
    let handler = Rc::new(handler);

    let listen_addr = "127.0.0.1:3000";
    let listener = TcpListener::bind(listen_addr).await.unwrap();
    println!("Listening on http://{listen_addr}");

    let local = LocalSet::new();
    loop {
        let (stream, peer_addr) = match listener.accept().await {
            Ok(x) => x,
            Err(err) => {
                eprintln!("Failed to accept connection: {err}");
                continue;
            }
        };

        let svc = Svc::new(lua.clone(), handler.clone(), peer_addr);
        local
            .run_until(async move {
                let result = ServerConnBuilder::new(LocalExec)
                    .http1()
                    .serve_connection(TokioIo::new(stream), svc)
                    .await;
                if let Err(err) = result {
                    eprintln!("Error serving connection: {err:?}");
                }
            })
            .await;
    }
}

#[derive(Clone, Copy, Debug)]
struct LocalExec;

impl<F> hyper::rt::Executor<F> for LocalExec
where
    F: Future + 'static, // not requiring `Send`
{
    fn execute(&self, fut: F) {
        tokio::task::spawn_local(fut);
    }
}
