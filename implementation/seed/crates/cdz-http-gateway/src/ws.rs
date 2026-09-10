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
    async fn deliver(&mut self, event: WsEvent) -> Vec<WsSend> {
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
