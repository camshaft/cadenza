//! The control-link client (`DESIGN-http-outpost.md` §2/§3, P3): the gateway DIALS a control server over a
//! WebSocket and receives its route table as a `cadenza-ast` frame.
//!
//! v0 is a minimal direct protocol (design D2, ahead of the federation hub): the outpost connects OUT to
//! the control server (outposts §2 — the edge dials in, so it needs no inbound reachability), and the server
//! pushes the route table (`http-route-table.cdz`, the same [`decode_route_table`](crate::codec) frame the
//! in-process mock control server ships in tests) as the first message on connect (§3). This is the wire
//! transport only — turning the received [`RouteFrame`]s into a live [`Router`](crate::gateway::Router) (or
//! seeding a router governing program) and handling live table UPDATES are later slices; here we prove the
//! dial + receive + decode. Handler component bytes are fetched by hash from the CAS separately (§3), not
//! over this link.
//!
//! Wasm-free: a `tokio` TCP dial + `tokio-tungstenite` client handshake + the `cadenza-ast` frame codec —
//! no wasmtime, so it sits outside the `host` feature.

use crate::codec::{RouteFrame, decode_route_table};
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
}
