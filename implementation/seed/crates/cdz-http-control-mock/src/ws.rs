//! The CONTROL-PLANE server (gateway-facing) — the real bidirectional ws protocol the STOCK gateway dials.
//! On connect it ships the mock's [`ControlConfig`](cdz_http_protocol::ControlConfig); it then reads inbound
//! [`ControlUp`](cdz_http_protocol::ControlUp) frames (a handler's `control.send`), records them, and sends
//! back the correlation-matched [`ControlDown`](cdz_http_protocol::ControlDown) reply the mock primed. Frames
//! are binary-AST (the `cdz-http-protocol` codec); the ws carries them as Binary messages.
//!
//! Each session registers an outbound-frame sender in the shared [`Sessions`] registry, and the session task
//! is a `select!` loop whose SOLE writer to the socket is that channel: an inbound reply is self-sent onto
//! the channel, and an admin-initiated push (a live-swap `ControlConfig` or an unsolicited `ControlDown`)
//! is sent by the admin layer onto the same channel. So both faces reach a live gateway through one path.

use crate::MockState;
use bytes::Bytes;
use cdz_http_protocol::{decode_control_up, encode_control_config, encode_control_down};
use futures_util::{SinkExt, StreamExt};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;

/// The live gateway sessions' outbound-frame senders, keyed by session id. Shared by the control-plane ws
/// server (which registers a session on connect) and the admin server (which pushes live-swaps / `ControlDown`
/// to a session). Sending a frame's bytes here writes it to that session's ws socket.
pub type Sessions = Arc<Mutex<HashMap<Bytes, mpsc::UnboundedSender<Bytes>>>>;

/// A fresh, empty session registry.
#[must_use]
pub fn new_sessions() -> Sessions {
    Arc::new(Mutex::new(HashMap::new()))
}

/// Push a frame's bytes to a single session's ws socket (an admin `ControlDown` / a targeted push). Returns
/// `false` if the session is unknown or its socket has closed.
#[must_use]
pub fn push_to(sessions: &Sessions, session: &Bytes, frame: Bytes) -> bool {
    sessions
        .lock()
        .expect("sessions mutex poisoned")
        .get(session)
        .is_some_and(|tx| tx.send(frame).is_ok())
}

/// Broadcast a frame's bytes to every connected session (a live-swap `ControlConfig`).
pub fn broadcast(sessions: &Sessions, frame: Bytes) {
    for tx in sessions.lock().expect("sessions mutex poisoned").values() {
        let _ = tx.send(frame.clone());
    }
}

/// Serve the control plane on `listener` until the task is dropped. One session per accepted socket.
pub async fn serve_control(
    listener: TcpListener,
    state: Arc<Mutex<MockState>>,
    sessions: Sessions,
) {
    let counter = Arc::new(AtomicU64::new(0));
    loop {
        let Ok((stream, _)) = listener.accept().await else {
            continue;
        };
        let session =
            Bytes::from(format!("sess-{}", counter.fetch_add(1, Ordering::Relaxed)).into_bytes());
        let state = state.clone();
        let sessions = sessions.clone();
        tokio::spawn(run_session(stream, session, state, sessions));
    }
}

/// Drive one gateway session: ws handshake → ship config → the read/push `select!` loop → on close, drop the
/// session from the registry and log the disconnect.
async fn run_session(
    stream: tokio::net::TcpStream,
    session: Bytes,
    state: Arc<Mutex<MockState>>,
    sessions: Sessions,
) {
    let Ok(ws) = tokio_tungstenite::accept_async(stream).await else {
        return;
    };
    let (mut sink, mut source) = ws.split();
    let (tx, mut rx) = mpsc::unbounded_channel::<Bytes>();

    // Register the session + ship the config (if the driver has set one). The config goes out via the loop's
    // push arm below.
    {
        let maybe_config = state
            .lock()
            .expect("state mutex poisoned")
            .on_connect(session.clone());
        sessions
            .lock()
            .expect("sessions mutex poisoned")
            .insert(session.clone(), tx.clone());
        if let Some(config) = maybe_config {
            let _ = tx.send(encode_control_config(&config));
        }
    }

    loop {
        tokio::select! {
            inbound = source.next() => match inbound {
                Some(Ok(Message::Binary(data))) => {
                    if let Some(up) = decode_control_up(&data) {
                        let reply = state.lock().expect("state mutex poisoned").record_control_up(up);
                        if let Some(down) = reply {
                            // Self-send the reply onto the channel so the push arm is the sole socket writer.
                            let _ = tx.send(encode_control_down(&down));
                        }
                    }
                }
                Some(Ok(Message::Close(_))) | None => break,
                Some(Ok(_)) => {} // ignore text / ping / pong / etc.
                Some(Err(_)) => break,
            },
            outbound = rx.recv() => match outbound {
                Some(frame) => {
                    // tungstenite 0.24 `Message::Binary` is `Vec<u8>`; the frame is `Bytes`.
                    if sink.send(Message::Binary(frame.to_vec())).await.is_err() {
                        break;
                    }
                }
                None => break,
            },
        }
    }

    sessions
        .lock()
        .expect("sessions mutex poisoned")
        .remove(&session);
    state
        .lock()
        .expect("state mutex poisoned")
        .on_disconnect(session);
}

#[cfg(test)]
mod tests {
    use super::*;
    use cdz_str::Str;
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message;

    fn hash(tag: &str) -> Bytes {
        let mut v = format!("cdz.prog.{tag}").into_bytes();
        v.resize(33, b'.');
        Bytes::from(v)
    }

    async fn boot() -> (std::net::SocketAddr, Arc<Mutex<MockState>>, Sessions) {
        let state = Arc::new(Mutex::new(MockState::new(HashMap::new())));
        let sessions = new_sessions();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(serve_control(listener, state.clone(), sessions.clone()));
        (addr, state, sessions)
    }

    async fn dial(
        addr: std::net::SocketAddr,
    ) -> tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>
    {
        let (ws, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/"))
            .await
            .unwrap();
        ws
    }

    #[tokio::test]
    async fn ships_the_config_on_connect() {
        let (addr, state, _sessions) = boot().await;
        state
            .lock()
            .unwrap()
            .set_config(cdz_http_protocol::ControlConfig {
                cas_url: Str::from("http://cas"),
                cas_credential: Bytes::new(),
                root_router: hash("router-hello"),
            });
        let mut ws = dial(addr).await;
        let Message::Binary(data) = ws.next().await.unwrap().unwrap() else {
            panic!("expected a binary config frame");
        };
        let config = cdz_http_protocol::decode_control_config(&data).expect("config decodes");
        assert_eq!(config.root_router, hash("router-hello"));
    }

    #[tokio::test]
    async fn a_control_up_gets_a_correlation_matched_reply() {
        let (addr, state, _sessions) = boot().await;
        // No config set → no initial frame. Prime a reply matched on path.
        state
            .lock()
            .unwrap()
            .set_config(cdz_http_protocol::ControlConfig {
                cas_url: Str::from("http://cas"),
                cas_credential: Bytes::new(),
                root_router: hash("r"),
            });
        state.lock().unwrap().prime_reply(crate::PrimedReply {
            match_program: None,
            match_path: Some(Str::from("/emit")),
            reply: Bytes::from_static(b"PONG"),
        });
        let mut ws = dial(addr).await;
        // Drain the initial config frame.
        let _ = ws.next().await.unwrap().unwrap();
        // Send a ControlUp on /emit.
        let up = cdz_http_protocol::ControlUp {
            program: hash("handler"),
            session: Bytes::from_static(b"ignored-the-mock-uses-its-own"),
            correlation: Bytes::from_static(b"corr-9"),
            payload: Bytes::from_static(b"ping"),
            request: cdz_http_protocol::RequestContext {
                method: Str::from("POST"),
                path: Str::from("/emit"),
                headers: vec![],
            },
        };
        ws.send(Message::Binary(
            cdz_http_protocol::encode_control_up(&up).to_vec(),
        ))
        .await
        .unwrap();
        // Receive the correlation-matched ControlDown reply.
        let Message::Binary(data) = ws.next().await.unwrap().unwrap() else {
            panic!("expected a binary reply frame");
        };
        let down = cdz_http_protocol::decode_control_down(&data).expect("down decodes");
        assert_eq!(down.correlation, Bytes::from_static(b"corr-9"));
        assert_eq!(down.payload, Bytes::from_static(b"PONG"));
        // The up was captured.
        assert_eq!(state.lock().unwrap().captured_up().len(), 1);
    }

    #[tokio::test]
    async fn a_broadcast_reaches_a_connected_session() {
        let (addr, _state, sessions) = boot().await;
        let mut ws = dial(addr).await;
        // Give the session a moment to register, then broadcast a frame.
        for _ in 0..50 {
            if !sessions.lock().unwrap().is_empty() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(2)).await;
        }
        broadcast(&sessions, Bytes::from_static(b"a-pushed-frame"));
        let Message::Binary(data) = ws.next().await.unwrap().unwrap() else {
            panic!("expected the pushed binary frame");
        };
        assert_eq!(data.as_slice(), b"a-pushed-frame");
    }
}
