//! The ADMIN server (driver-facing) — a hyper HTTP/1 server whose request/response BODIES are binary-AST
//! [`AdminCommand`]/[`AdminReply`] values (NOT JSON; HTTP is only the byte transport). The driver `POST`s an
//! `AdminCommand` to `/admin` and reads back an `AdminReply`; `GET /healthz` is the readiness probe the boot
//! fixture blocks on.
//!
//! It dispatches each command against the shared [`MockState`] (behind a `std::sync::Mutex` — every op is a
//! short sync critical section, never held across `.await`). Cross-face effects that must reach a live
//! gateway session (pushing a root-router live-swap or an unsolicited `ControlDown`) update the state here;
//! the control-plane ws layer applies them to connected sessions (wired in a following slice).

use crate::admin::{AdminCommand, AdminReply, decode_command, encode_reply};
use crate::ws::{Sessions, broadcast, push_to};
use crate::{MockState, PrimedReply};
use bytes::Bytes;
use cdz_http_protocol::{ControlConfig, ControlDown, ControlFrame, FrameCodec, encode_control_up};
use cdz_str::Str;
use http_body_util::{BodyExt, Full};
use hyper::service::service_fn;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use std::convert::Infallible;
use std::sync::{Arc, Mutex};
use tokio::net::TcpListener;

/// The shared context an admin request handler needs: the mock state, plus the base CAS coordinates the
/// mock ships in a `ControlConfig` (learned at startup — the driver spun up the CAS on a known socket).
#[derive(Clone)]
pub struct AdminCtx {
    pub state: Arc<Mutex<MockState>>,
    /// The live gateway sessions (shared with the control-plane ws server) so a live-swap / push reaches them.
    pub sessions: Sessions,
    pub cas_url: Str,
    pub cas_credential: Bytes,
    /// The tagged control-link frame codec (config/up/down ids) — pushes to gateway sessions go out TAGGED.
    pub codec: FrameCodec,
}

/// Serve the admin API on `listener` until the task is dropped. One connection per accepted socket, each on
/// its own task; HTTP/1, no upgrades.
pub async fn serve_admin(listener: TcpListener, ctx: AdminCtx) {
    loop {
        let Ok((stream, _)) = listener.accept().await else {
            continue;
        };
        let ctx = ctx.clone();
        tokio::spawn(async move {
            let io = TokioIo::new(stream);
            let service = service_fn(move |req| handle(req, ctx.clone()));
            let _ = hyper::server::conn::http1::Builder::new()
                .serve_connection(io, service)
                .await;
        });
    }
}

/// Route one admin request. `GET /healthz` → readiness; `POST /admin` → decode an [`AdminCommand`], dispatch
/// it, and return the [`AdminReply`] bytes; anything else → 404. A malformed command body → 400.
async fn handle(
    req: Request<hyper::body::Incoming>,
    ctx: AdminCtx,
) -> Result<Response<Full<Bytes>>, Infallible> {
    match (req.method(), req.uri().path()) {
        (&Method::GET, "/healthz") => Ok(text(StatusCode::OK, "ok")),
        (&Method::POST, "/admin") => {
            let body = match req.into_body().collect().await {
                Ok(b) => b.to_bytes(),
                Err(_) => return Ok(text(StatusCode::BAD_REQUEST, "body read error")),
            };
            match decode_command(&body) {
                Some(cmd) => {
                    let reply = dispatch(cmd, &ctx);
                    Ok(bin(StatusCode::OK, encode_reply(&reply)))
                }
                None => Ok(text(StatusCode::BAD_REQUEST, "malformed AdminCommand")),
            }
        }
        _ => Ok(text(StatusCode::NOT_FOUND, "not found")),
    }
}

/// Apply one [`AdminCommand`] to the mock state and produce its [`AdminReply`]. State-only ops hold the
/// state lock for a short sync section; a live-swap / push RELEASES the state lock before locking the session
/// registry (never both at once, so the two mutexes have no lock-order coupling). No `.await` anywhere.
fn dispatch(cmd: AdminCommand, ctx: &AdminCtx) -> AdminReply {
    match cmd {
        AdminCommand::SetProgram { name, hash } => {
            ctx.state
                .lock()
                .expect("state mutex poisoned")
                .add_program(name, hash);
            AdminReply::Ok
        }
        AdminCommand::SetRootRouter { program } => {
            let mut st = ctx.state.lock().expect("state mutex poisoned");
            match st.resolve(&program) {
                Some(root_router) => {
                    st.set_config(ControlConfig {
                        cas_url: ctx.cas_url.clone(),
                        cas_credential: ctx.cas_credential.clone(),
                        root_router,
                    });
                    AdminReply::Ok
                }
                None => unresolvable(&program),
            }
        }
        AdminCommand::PushRootRouter { program } => {
            // Under the state lock: resolve + swap the config, and capture the new config to ship. Then drop
            // the state lock and broadcast the fresh ControlConfig to every live session (the live-swap).
            let frame = {
                let mut st = ctx.state.lock().expect("state mutex poisoned");
                match st.resolve(&program) {
                    Some(root_router) => {
                        st.push_root_router(root_router);
                        st.config()
                            .map(|c| ctx.codec.encode(&ControlFrame::Config(c.clone())))
                    }
                    None => return unresolvable(&program),
                }
            };
            if let Some(frame) = frame {
                broadcast(&ctx.sessions, frame);
            }
            AdminReply::Ok
        }
        AdminCommand::PrimeReply { match_path, reply } => {
            ctx.state
                .lock()
                .expect("state mutex poisoned")
                .prime_reply(PrimedReply {
                    match_program: None,
                    match_path,
                    reply,
                });
            AdminReply::Ok
        }
        AdminCommand::PushDown { session, payload } => {
            // An unsolicited push (empty correlation): record it, then deliver it to the target session.
            let down = ControlDown {
                session: session.clone(),
                correlation: Bytes::new(),
                payload,
            };
            ctx.state
                .lock()
                .expect("state mutex poisoned")
                .record_control_down(&down);
            let _ = push_to(
                &ctx.sessions,
                &session,
                ctx.codec.encode(&ControlFrame::Down(down)),
            );
            AdminReply::Ok
        }
        AdminCommand::Reset => {
            ctx.state.lock().expect("state mutex poisoned").reset();
            AdminReply::Ok
        }
        AdminCommand::GetControlUps => AdminReply::ControlUps {
            ups: ctx
                .state
                .lock()
                .expect("state mutex poisoned")
                .captured_up()
                .iter()
                .map(encode_control_up)
                .collect(),
        },
    }
}

fn unresolvable(program: &str) -> AdminReply {
    AdminReply::Error {
        message: Str::from(format!("unresolvable program '{program}'").as_str()),
    }
}

fn text(status: StatusCode, msg: &str) -> Response<Full<Bytes>> {
    Response::builder()
        .status(status)
        .body(Full::new(Bytes::copy_from_slice(msg.as_bytes())))
        .expect("static response builds")
}

fn bin(status: StatusCode, body: Bytes) -> Response<Full<Bytes>> {
    Response::builder()
        .status(status)
        .body(Full::new(body))
        .expect("response builds")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::admin::{decode_reply, encode_command};
    use http_body_util::Empty;
    use std::collections::HashMap;

    fn hash(tag: &str) -> Bytes {
        let mut v = format!("cdz.prog.{tag}").into_bytes();
        v.resize(33, b'.');
        Bytes::from(v)
    }

    /// Boot an admin server on an ephemeral loopback port; return its address + the shared state handle.
    async fn boot() -> (std::net::SocketAddr, Arc<Mutex<MockState>>) {
        let programs = HashMap::from([(Str::from("router-hello"), hash("router-hello"))]);
        let state = Arc::new(Mutex::new(MockState::new(programs)));
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let ctx = AdminCtx {
            state: state.clone(),
            sessions: crate::ws::new_sessions(),
            cas_url: Str::from("http://127.0.0.1:9/cas"),
            cas_credential: Bytes::from_static(b"tok"),
            codec: FrameCodec::new(
                Bytes::from_static(b"c"),
                Bytes::from_static(b"u"),
                Bytes::from_static(b"d"),
            ),
        };
        tokio::spawn(serve_admin(listener, ctx));
        (addr, state)
    }

    /// POST an AdminCommand to the running server and decode the AdminReply.
    async fn admin(addr: std::net::SocketAddr, cmd: &AdminCommand) -> AdminReply {
        use hyper::client::conn::http1;
        let stream = tokio::net::TcpStream::connect(addr).await.unwrap();
        let (mut sender, conn) = http1::handshake(TokioIo::new(stream)).await.unwrap();
        tokio::spawn(conn);
        let req = Request::builder()
            .method(Method::POST)
            .uri("/admin")
            .body(Full::new(encode_command(cmd)))
            .unwrap();
        let resp = sender.send_request(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        decode_reply(&body).expect("reply decodes")
    }

    #[tokio::test]
    async fn set_root_router_over_a_socket_resolves_the_name_and_configures() {
        let (addr, state) = boot().await;
        let reply = admin(
            addr,
            &AdminCommand::SetRootRouter {
                program: Str::from("router-hello"),
            },
        )
        .await;
        assert_eq!(reply, AdminReply::Ok);
        // The mock now ships a config carrying the resolved hash + the startup CAS coordinates.
        let st = state.lock().unwrap();
        let config = st.config().expect("config set");
        assert_eq!(config.root_router, hash("router-hello"));
        assert_eq!(config.cas_url, "http://127.0.0.1:9/cas");
    }

    #[tokio::test]
    async fn an_unresolvable_program_is_an_error_reply() {
        let (addr, _state) = boot().await;
        let reply = admin(
            addr,
            &AdminCommand::SetRootRouter {
                program: Str::from("does-not-exist"),
            },
        )
        .await;
        match reply {
            AdminReply::Error { message } => assert!(message.contains("does-not-exist")),
            other => panic!("expected Error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn prime_reply_then_get_control_ups_round_trips_over_the_socket() {
        let (addr, state) = boot().await;
        // Prime a reply (state-only effect) — Ok.
        assert_eq!(
            admin(
                addr,
                &AdminCommand::PrimeReply {
                    match_path: Some(Str::from("/emit")),
                    reply: Bytes::from_static(b"PONG"),
                },
            )
            .await,
            AdminReply::Ok
        );
        // Simulate a captured control.send (as the ws layer would) so GetControlUps has something to return.
        {
            let mut st = state.lock().unwrap();
            st.record_control_up(cdz_http_protocol::ControlUp {
                program: hash("handler"),
                session: Bytes::from_static(b"s1"),
                correlation: Bytes::from_static(b"c1"),
                payload: Bytes::from_static(b"ping"),
                request: cdz_http_protocol::RequestContext {
                    method: Str::from("POST"),
                    path: Str::from("/emit"),
                    headers: vec![],
                },
            });
        }
        let reply = admin(addr, &AdminCommand::GetControlUps).await;
        let AdminReply::ControlUps { ups } = reply else {
            panic!("expected ControlUps");
        };
        assert_eq!(ups.len(), 1);
        let up = cdz_http_protocol::decode_control_up(&ups[0]).expect("up decodes");
        assert_eq!(up.request.path, "/emit");
        assert_eq!(up.correlation, Bytes::from_static(b"c1"));
    }

    #[tokio::test]
    async fn healthz_is_ready() {
        let (addr, _state) = boot().await;
        use hyper::client::conn::http1;
        let stream = tokio::net::TcpStream::connect(addr).await.unwrap();
        let (mut sender, conn) = http1::handshake(TokioIo::new(stream)).await.unwrap();
        tokio::spawn(conn);
        let req = Request::builder()
            .uri("/healthz")
            .body(Empty::<Bytes>::new())
            .unwrap();
        let resp = sender.send_request(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }
}
