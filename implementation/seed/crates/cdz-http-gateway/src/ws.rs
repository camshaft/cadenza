//! The per-connection WebSocket session (`DESIGN-http-outpost.md` §6, P4).
//!
//! A WebSocket endpoint is a session spawned PER CONNECTION (not per request): on the upgrade the router
//! spawns it, the edge folds a `ws-event` `Connect` into it, then a `Frame` per inbound WebSocket frame,
//! then `Disconnect` on close. The session emits `ws-send` effects (requests on the `ws-send` contract-id)
//! to PUSH frames back; it stays alive across frames until it `Break`s or the connection closes.
//!
//! [`WsSession`] is the socket-independent driver of that — generic over [`ProgramStore`], so it drives a
//! wasmtime session in production and a native session in tests. The hyper WebSocket upgrade + framing that
//! feeds it inbound frames and writes its `ws-send` frames to the socket is a later slice (P4c); the session
//! reducer itself is unchanged either way.

use crate::codec::{WsEvent, WsSend, decode_ws_send, encode_ws_event};
use cdz_platform::{
    Bytes, ContractId, HostId, Message, Origin, Outcome, ProgramHash, ProgramStore, Reducer,
    ReducerId, ReducerKind, SpawnContext,
};

/// A live per-connection WebSocket session: the spawned session reducer plus the envelope metadata each
/// delivered `ws-event` carries and the `ws-send` contract-id that identifies the frames it pushes back.
pub struct WsSession {
    reducer: Box<dyn Reducer>,
    conn: Bytes,
    host: HostId,
    router: ReducerId,
    /// The contract-id delivered as each `ws-event`'s `Message.id`.
    event_contract: ContractId,
    /// The contract-id a session's emitted request carries when it is a `ws-send` (a frame to push); other
    /// emitted requests are ignored by the ws edge.
    send_contract: ContractId,
    open: bool,
}

impl WsSession {
    /// Open a session: spawn `program` (a fresh per-connection instance keyed on `session_id`) and fold the
    /// `Connect` event. Returns the session and any frames it pushes on connect, or `None` if the store
    /// cannot instantiate the program.
    #[allow(clippy::too_many_arguments)]
    pub async fn open(
        store: &dyn ProgramStore,
        program: ProgramHash,
        session_id: &[u8],
        conn: Bytes,
        host: HostId,
        router: ReducerId,
        event_contract: ContractId,
        send_contract: ContractId,
    ) -> Option<(WsSession, Vec<WsSend>)> {
        let reducer = store
            .spawn(
                program,
                SpawnContext {
                    id: ReducerId::of(session_id),
                    kind: ReducerKind::Ordinary,
                    limits: None,
                },
            )
            .await?;
        let mut session = WsSession {
            reducer,
            conn: conn.clone(),
            host,
            router,
            event_contract,
            send_contract,
            open: true,
        };
        let sends = session.deliver(WsEvent::Connect { conn }).await;
        Some((session, sends))
    }

    /// Fold one inbound frame, returning the frames the session pushes in response.
    pub async fn on_frame(&mut self, data: Bytes) -> Vec<WsSend> {
        let conn = self.conn.clone();
        self.deliver(WsEvent::Frame { conn, data }).await
    }

    /// Fold the disconnect (the session closes), returning any final frames it pushes.
    pub async fn close(&mut self) -> Vec<WsSend> {
        let conn = self.conn.clone();
        self.deliver(WsEvent::Disconnect { conn }).await
    }

    /// Whether the session is still open (it closes when the reducer `Break`s or after `close`).
    #[must_use]
    pub fn is_open(&self) -> bool {
        self.open
    }

    /// Deliver one `ws-event` to the session and collect the `ws-send` frames it emits (its requests on the
    /// `ws-send` contract-id, decoded). A `Break` outcome closes the session.
    ///
    /// A CLOSED session folds nothing further: once the reducer has `Break`ed, delivering another event
    /// (e.g. the frame loop's final `Disconnect` after a guest closed itself on a `Frame`) would be a
    /// spurious extra `on_message` into a session that already ended — it could emit stray pushes or, for a
    /// guest that assumes a single close, misbehave. So a fold on a closed session is a no-op.
    async fn deliver(&mut self, event: WsEvent) -> Vec<WsSend> {
        if !self.open {
            return Vec::new();
        }
        let (requests, outcome) = self
            .reducer
            .on_message(Message {
                id: self.event_contract,
                payload: encode_ws_event(&event),
                from: Origin {
                    reducer: self.router,
                    host: self.host,
                },
                continuation_token: Bytes::new(),
            })
            .await;
        if matches!(outcome, Outcome::Break { .. }) {
            self.open = false;
        }
        requests
            .into_iter()
            .filter(|r| r.id == self.send_contract)
            .filter_map(|r| decode_ws_send(&r.payload))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::encode_ws_send;
    use async_trait::async_trait;
    use cdz_platform::testing::program::Store;
    use cdz_platform::{Notification, Request, Response};

    fn event_contract() -> ContractId {
        ContractId::of(b"cdz-platform.ws.event")
    }
    fn send_contract() -> ContractId {
        ContractId::of(b"cdz-platform.ws.send")
    }

    /// A native WS session that echoes every inbound frame back as a `ws-send`, and closes on disconnect.
    struct WsEcho;
    #[async_trait]
    impl Reducer for WsEcho {
        async fn on_message(&mut self, m: Message) -> (Vec<Request>, Outcome) {
            match crate::codec::decode_ws_event(&m.payload) {
                Some(WsEvent::Frame { conn, data }) => {
                    // Echo the frame back as a ws-send request; keep the session open.
                    let req = Request {
                        id: send_contract(),
                        payload: encode_ws_send(&WsSend { conn, data }),
                        continuation_token: Bytes::new(),
                        deadline: None,
                    };
                    (vec![req], Outcome::Continue)
                }
                Some(WsEvent::Disconnect { .. }) => (
                    vec![],
                    Outcome::Break {
                        schema: ContractId::of(b"cdz-platform.ws.event.........."),
                        reason: Bytes::new(),
                    },
                ),
                // Connect / a malformed event: nothing to push, stay open.
                _ => (vec![], Outcome::Continue),
            }
        }
        async fn on_response(&mut self, _r: Response) -> (Vec<Request>, Outcome) {
            (vec![], Outcome::Continue)
        }
        async fn on_notification(&mut self, _n: Notification) -> (Vec<Request>, Outcome) {
            (vec![], Outcome::Continue)
        }
    }

    #[tokio::test]
    async fn a_ws_session_folds_frames_and_collects_pushes() {
        let program = ProgramHash::of(b"ws-echo");
        let mut store = Store::new();
        store.register(program, || Box::new(WsEcho));

        let (mut session, on_connect) = WsSession::open(
            &store,
            program,
            b"sess-1",
            Bytes::from_static(b"conn-1"),
            HostId::of(b"h"),
            ReducerId::of(b"r"),
            event_contract(),
            send_contract(),
        )
        .await
        .expect("session opens");
        assert!(on_connect.is_empty(), "Connect pushes nothing");
        assert!(session.is_open());

        // A frame is echoed back as one push, carrying the connection id + the frame data.
        let pushes = session.on_frame(Bytes::from_static(b"hello")).await;
        assert_eq!(pushes.len(), 1);
        assert_eq!(pushes[0].conn, Bytes::from_static(b"conn-1"));
        assert_eq!(pushes[0].data, Bytes::from_static(b"hello"));
        assert!(session.is_open(), "an echo keeps the session open");

        // Disconnect closes the session.
        let _ = session.close().await;
        assert!(!session.is_open(), "disconnect closes the session");
    }

    #[tokio::test]
    async fn a_closed_session_folds_no_further_events() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicUsize, Ordering};

        // A session that closes itself on its first `Frame` (a guest that ends the connection mid-stream),
        // counting every `on_message` so the test can prove no fold happens once it has closed.
        struct BreakOnFrame(Arc<AtomicUsize>);
        #[async_trait]
        impl Reducer for BreakOnFrame {
            async fn on_message(&mut self, m: Message) -> (Vec<Request>, Outcome) {
                self.0.fetch_add(1, Ordering::Relaxed);
                match crate::codec::decode_ws_event(&m.payload) {
                    Some(WsEvent::Frame { .. }) => (
                        vec![],
                        Outcome::Break {
                            schema: send_contract(),
                            reason: Bytes::new(),
                        },
                    ),
                    _ => (vec![], Outcome::Continue),
                }
            }
            async fn on_response(&mut self, _r: Response) -> (Vec<Request>, Outcome) {
                (vec![], Outcome::Continue)
            }
            async fn on_notification(&mut self, _n: Notification) -> (Vec<Request>, Outcome) {
                (vec![], Outcome::Continue)
            }
        }

        let calls = Arc::new(AtomicUsize::new(0));
        let program = ProgramHash::of(b"break-on-frame");
        let mut store = Store::new();
        let calls_for_factory = Arc::clone(&calls);
        store.register(program, move || {
            Box::new(BreakOnFrame(Arc::clone(&calls_for_factory)))
        });

        let (mut session, _on_connect) = WsSession::open(
            &store,
            program,
            b"sess",
            Bytes::from_static(b"conn"),
            HostId::of(b"h"),
            ReducerId::of(b"r"),
            event_contract(),
            send_contract(),
        )
        .await
        .expect("session opens");
        assert_eq!(calls.load(Ordering::Relaxed), 1, "Connect folded once");
        assert!(session.is_open());

        // The first frame closes the session (fold #2).
        let _ = session.on_frame(Bytes::from_static(b"bye")).await;
        assert!(!session.is_open(), "the guest closed itself on the frame");
        assert_eq!(calls.load(Ordering::Relaxed), 2);

        // Any further fold on the closed session is a no-op — no push AND no extra `on_message` (the frame
        // loop's unconditional final `close()` must not deliver a spurious `Disconnect` after a Break).
        assert!(session.on_frame(Bytes::from_static(b"x")).await.is_empty());
        assert!(session.close().await.is_empty());
        assert_eq!(
            calls.load(Ordering::Relaxed),
            2,
            "a closed session must not fold any further event into the reducer"
        );
    }

    #[tokio::test]
    async fn an_unknown_program_opens_no_session() {
        let store = Store::new();
        let opened = WsSession::open(
            &store,
            ProgramHash::of(b"absent"),
            b"s",
            Bytes::from_static(b"c"),
            HostId::of(b"h"),
            ReducerId::of(b"r"),
            event_contract(),
            send_contract(),
        )
        .await;
        assert!(opened.is_none());
    }
}

#[cfg(all(test, feature = "host"))]
mod host_e2e {
    use super::*;
    use crate::wasm::{spawn_epoch_ticker, wasm_store};
    use cdz_platform::{BlobStore, InMemoryBlobStore, ProgramStore};
    use std::sync::Arc;

    /// THE WS END-TO-END PAYOFF (`DESIGN-http-outpost.md` §6): a REAL content-addressed wasm ws-session guest
    /// driven through [`WsSession`] over the wasmtime store — a `Connect`/`Frame`/`Disconnect` folded by the
    /// guest, its `ws-send` push decoded back. This proves the LAST unproven seam of the outpost: BOTH ws
    /// codec directions across the Cadenza<->Rust boundary at once — the guest `Value.decode`s the gateway's
    /// [`encode_ws_event`](crate::codec::encode_ws_event) (forward), and the gateway
    /// [`decode_ws_send`](crate::codec::decode_ws_send)s the guest's `Value.encode`d push (reverse) — AND
    /// reconciles the guest's raw 33-byte `ws-send` contract-id marker with the session's `send_contract`
    /// (a request-emitting guest, unlike the http handlers which answer via a `Break` reason). The guest
    /// component imports the value-heap runtime (`cadenza:runtime/heap@…`), which the host COMPOSES from the
    /// CAS by hash, so the runtime + its NFC dep are seeded alongside the guest. All three paths come from
    /// env vars the fleet nix check sets; the test skips cleanly when any is unset so `cargo test
    /// --features host` passes without them.
    #[tokio::test]
    async fn wasm_ws_session_echoes_a_frame() {
        let (Ok(guest_path), Ok(runtime_path), Ok(nfc_path)) = (
            std::env::var("CDZ_HTTP_WS_ECHO_WASM"),
            std::env::var("CDZ_HTTP_RUNTIME_WASM"),
            std::env::var("CDZ_HTTP_NFC_WASM"),
        ) else {
            eprintln!(
                "wasm_ws_session_echoes_a_frame: CDZ_HTTP_WS_ECHO_WASM/RUNTIME_WASM/NFC_WASM unset — \
                 skipping (the nix check sets all three)"
            );
            return;
        };
        let guest = std::fs::read(&guest_path).expect("read ws-echo guest wasm");

        // Seed the value-heap runtime + its NFC dep (so the host composes the guest's `cadenza:runtime/heap`
        // import by hash) and the ws-session guest itself into the content store.
        let mut cas = InMemoryBlobStore::new();
        for dep in [&runtime_path, &nfc_path] {
            cas.put(Bytes::from(std::fs::read(dep).expect("read dep component")))
                .await;
        }
        cas.put(Bytes::from(guest.clone())).await;
        let cas: Arc<dyn BlobStore> = Arc::new(cas);
        let program = ProgramHash::of(&guest);
        let store: Arc<dyn ProgramStore> =
            Arc::new(wasm_store(Arc::clone(&cas)).expect("wasm store"));
        let _ticker = spawn_epoch_ticker(store.as_ref());

        // The guest emits its `ws-send` push on the raw 33-byte marker `b"cdz-platform.ws.send............."`
        // (Hash::LEN); the host surfaces an emitted request's contract as
        // `ContractId::from_hash(Hash::from_bytes(<those 33 bytes>))`, which `ContractId::try_from` of the
        // same 33 bytes reproduces exactly — so the session's `r.id == send_contract` filter keeps the push.
        let send_contract = ContractId::try_from(&b"cdz-platform.ws.send............."[..])
            .expect("33-byte ws-send marker");
        // The guest dispatches on the DECODED event, ignoring the event contract-id, so any id serves here.
        let event_contract = ContractId::of(b"cdz-platform.ws.event");

        let (mut session, on_connect) = WsSession::open(
            store.as_ref(),
            program,
            b"ws-sess-1",
            Bytes::from_static(b"conn-1"),
            HostId::of(b"edge-host"),
            ReducerId::of(b"router"),
            event_contract,
            send_contract,
        )
        .await
        .expect("ws session opens");
        assert!(on_connect.is_empty(), "the guest pushes nothing on Connect");
        assert!(session.is_open());

        // A frame is echoed straight back as one push, carrying the connection id + the frame bytes.
        let pushes = session.on_frame(Bytes::from_static(b"hello ws")).await;
        assert_eq!(pushes.len(), 1, "the guest echoes one frame back");
        assert_eq!(pushes[0].conn, Bytes::from_static(b"conn-1"));
        assert_eq!(pushes[0].data, Bytes::from_static(b"hello ws"));
        assert!(session.is_open(), "an echo keeps the session open");

        // Disconnect: the guest `Break`s, closing the session.
        let _ = session.close().await;
        assert!(!session.is_open(), "the guest Breaks on Disconnect");
    }
}
