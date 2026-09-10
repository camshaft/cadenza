//! The control-link client (`DESIGN-http-outpost.md` §2/§3, P3): the gateway DIALS a control server over a
//! WebSocket and receives its route table as a `cadenza-ast` frame.
//!
//! v0 is a minimal direct protocol (design D2, ahead of the federation hub): the outpost connects OUT to
//! the control server (outposts §2 — the edge dials in, so it needs no inbound reachability), and the server
//! pushes the route table (`http-route-table.cdz`, the same [`decode_route_table`](crate::codec) frame the
//! in-process mock control server ships in tests) as the first message on connect (§3), then each update.
//! [`fetch_route_table`] does the one-shot dial+read (boot-time seed); [`run_control_link`] keeps the link
//! open and swaps a [`DynamicRouter`](crate::gateway::DynamicRouter)'s live table on every pushed frame (a
//! route-table UPDATE lands without a restart). Handler component bytes are fetched by hash from the CAS
//! separately (§3), not over this link.
//!
//! Wasm-free: a `tokio` TCP dial + `tokio-tungstenite` client handshake + the `cadenza-ast` frame codec —
//! no wasmtime, so it sits outside the `host` feature.

use crate::codec::{RouteFrame, decode_route_table};
use crate::gateway::RouteTableCell;
use bytes::Bytes;
use futures_util::StreamExt;
use std::net::SocketAddr;
use tokio_tungstenite::tungstenite::Message;

/// Dial the control server at `addr` over a WebSocket and read its route-table frame, decoded into
/// [`RouteFrame`]s. Reads the FIRST data frame the server pushes on connect (the route table, §3), skipping
/// protocol control frames (ping/pong). Returns `None` on any failure — the TCP dial or ws handshake fails,
/// the connection closes before a data frame arrives, or the frame is not a decodable route table — so a
/// caller can fall back (retry, or a locally-seeded table) rather than crash.
pub async fn fetch_route_table(addr: SocketAddr) -> Option<Vec<RouteFrame>> {
    let stream = tokio::net::TcpStream::connect(addr).await.ok()?;
    let (mut ws, _resp) = tokio_tungstenite::client_async(format!("ws://{addr}/"), stream)
        .await
        .ok()?;
    // The route table is the first data frame the control server pushes on connect (§3). tungstenite handles
    // ping/pong itself, but a stray control frame surfacing here is skipped rather than treated as the table.
    loop {
        match ws.next().await {
            Some(Ok(Message::Binary(bytes))) => return decode_route_table(&bytes),
            Some(Ok(Message::Text(text))) => return decode_route_table(text.as_bytes()),
            Some(Ok(_)) => continue, // a control frame (ping/pong/close-less) — keep waiting for the table
            _ => return None,        // stream error or closed before any data frame
        }
    }
}

/// Dial the control server and keep the link OPEN, swapping `cell`'s route-table frame LIVE on every frame
/// the server pushes (the initial table on connect, then each update, §3) — the running counterpart of
/// [`fetch_route_table`]. A [`DynamicRouter`](crate::gateway::DynamicRouter)'s
/// [`table_cell`](crate::gateway::DynamicRouter::table_cell) is the `cell`, so a pushed update is folded by
/// the very next request with no restart. Each frame is VALIDATED as a route table before it is stored (a
/// malformed push is ignored, not allowed to poison routing); the RAW frame bytes are stored (the router
/// guest re-decodes them). Returns when the dial fails or the connection closes — the caller (e.g. a
/// supervising task) decides whether to redial. Best-effort: this never panics and never disturbs the
/// currently-serving table on a bad frame.
pub async fn run_control_link(addr: SocketAddr, cell: RouteTableCell) {
    let Ok(stream) = tokio::net::TcpStream::connect(addr).await else {
        return;
    };
    let Ok((mut ws, _resp)) =
        tokio_tungstenite::client_async(format!("ws://{addr}/"), stream).await
    else {
        return;
    };
    while let Some(Ok(msg)) = ws.next().await {
        let frame = match msg {
            Message::Binary(bytes) => Bytes::from(bytes),
            Message::Text(text) => Bytes::from(text.into_bytes()),
            _ => continue, // control frame — no table update
        };
        // Only a well-formed route table is allowed to swap the live table; a malformed push is ignored.
        if decode_route_table(&frame).is_some() {
            *cell.lock().expect("route table lock") = frame;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::{Method, encode_route_table};
    use bytes::Bytes;
    use futures_util::SinkExt;

    fn sample_table() -> Vec<RouteFrame> {
        vec![
            RouteFrame {
                method: Method::Get,
                path: "/".to_string(),
                handler: Bytes::from_static(b"cdz-http.handler.root............"),
                contract: Bytes::from_static(b"cdz-platform.http.request........"),
            },
            RouteFrame {
                method: Method::Post,
                path: "/mcp".to_string(),
                handler: Bytes::from_static(b"cdz-http.handler.mcp............."),
                contract: Bytes::from_static(b"cdz-platform.http.request........"),
            },
        ]
    }

    /// A stub control server: accept ONE ws connection and push the route-table frame, mirroring the real
    /// control server's connect-time push (§3).
    async fn serve_one_frame(listener: tokio::net::TcpListener, frame: Bytes) {
        let (stream, _peer) = listener.accept().await.expect("accept");
        let mut ws = tokio_tungstenite::accept_async(stream)
            .await
            .expect("server handshake");
        ws.send(Message::Binary(frame.to_vec()))
            .await
            .expect("push frame");
        // Keep the connection briefly so the client reads the frame before the socket drops.
        let _ = ws.close(None).await;
    }

    #[tokio::test]
    async fn dials_and_decodes_the_control_servers_route_table() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let addr = listener.local_addr().expect("addr");
        let frame = encode_route_table(&sample_table());
        tokio::spawn(serve_one_frame(listener, frame));

        let routes = fetch_route_table(addr).await.expect("route table arrives");
        assert_eq!(routes, sample_table());
    }

    #[tokio::test]
    async fn a_dial_to_a_dead_address_is_none() {
        // An unbound port: nothing is listening → the dial fails → None (the caller falls back).
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let addr = listener.local_addr().expect("addr");
        drop(listener); // free the port so the connect is refused
        assert!(fetch_route_table(addr).await.is_none());
    }

    /// A control server that sends a non-route-table frame → the client decodes nothing (None), not a panic.
    #[tokio::test]
    async fn a_malformed_frame_is_none() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let addr = listener.local_addr().expect("addr");
        tokio::spawn(serve_one_frame(
            listener,
            Bytes::from_static(b"not a frame"),
        ));
        assert!(fetch_route_table(addr).await.is_none());
    }

    /// `run_control_link` swaps the live cell on EVERY pushed frame: a server that pushes two tables in a row
    /// leaves the cell holding the second — proving a route-table UPDATE lands without a reconnect. A garbage
    /// push in between is ignored (does not poison the cell).
    #[tokio::test]
    async fn run_control_link_swaps_the_table_live() {
        use crate::gateway::RouteTableCell;
        use std::sync::{Arc, Mutex};

        let table_a = encode_route_table(&[RouteFrame {
            method: Method::Get,
            path: "/a".to_string(),
            handler: Bytes::from_static(b"cdz-http.handler.a.............."),
            contract: Bytes::from_static(b"cdz-platform.http.request........"),
        }]);
        let table_b = encode_route_table(&[RouteFrame {
            method: Method::Get,
            path: "/b".to_string(),
            handler: Bytes::from_static(b"cdz-http.handler.b.............."),
            contract: Bytes::from_static(b"cdz-platform.http.request........"),
        }]);

        // A stub control server: push table_a, then a GARBAGE frame (must be ignored), then table_b, close.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let addr = listener.local_addr().expect("addr");
        let (a, b) = (table_a.clone(), table_b.clone());
        tokio::spawn(async move {
            let (s, _) = listener.accept().await.expect("accept");
            let mut ws = tokio_tungstenite::accept_async(s).await.expect("handshake");
            ws.send(Message::Binary(a.to_vec())).await.expect("push a");
            ws.send(Message::Binary(b"garbage".to_vec()))
                .await
                .expect("push garbage");
            ws.send(Message::Binary(b.to_vec())).await.expect("push b");
            let _ = ws.close(None).await;
        });

        // The cell starts empty; run the link to completion (it returns when the server closes).
        let cell: RouteTableCell = Arc::new(Mutex::new(Bytes::new()));
        run_control_link(addr, Arc::clone(&cell)).await;

        // The live cell reflects the LAST valid table pushed (table_b) — the garbage push was ignored.
        assert_eq!(*cell.lock().expect("lock"), table_b);
        assert_ne!(*cell.lock().expect("lock"), table_a);
    }

    #[tokio::test]
    async fn run_control_link_to_a_dead_address_returns() {
        use crate::gateway::RouteTableCell;
        use std::sync::{Arc, Mutex};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let addr = listener.local_addr().expect("addr");
        drop(listener);
        let cell: RouteTableCell = Arc::new(Mutex::new(Bytes::new()));
        // A failed dial returns promptly (no panic, no hang) — the caller decides whether to redial.
        run_control_link(addr, Arc::clone(&cell)).await;
        assert!(cell.lock().expect("lock").is_empty());
    }
}
