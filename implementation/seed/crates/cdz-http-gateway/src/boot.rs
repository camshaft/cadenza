//! Boot-from-control (`DESIGN-http-outpost-drive-contract.md` §3/§4) — the dumb gateway's startup path.
//!
//! The gateway is told ONLY where to serve HTTP and where the control server is. [`run`] dials a single
//! persistent WebSocket to control, reads the [`ControlConfig`] control ships on connect (CAS url +
//! credential + root-router `ProgramHash`), builds the content-addressed store client from it, binds the
//! HTTP listener, and serves.
//!
//! This is the FIRST boot slice (the bootable stub the conformance harness's driver spins up): it dials
//! control, applies the config, and serves HTTP floors, keeping the control link open. Driving the
//! control-shipped root-router program per request (fetch it from the CAS by hash, run its mailbox drive
//! loop, dispatch to handlers, resolve effects) + live-swapping the router on a pushed config are the next
//! slices; until then every request gets the boot floor below.

use crate::HttpBlobStore;
use bytes::Bytes;
use cdz_http_protocol::{ControlConfig, ControlFrame, FrameCodec, decode_control_config};
use futures_util::StreamExt;
use http_body_util::Full;
use hyper::service::service_fn;
use hyper::{Response, StatusCode};
use hyper_util::rt::TokioIo;
use std::convert::Infallible;
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::tungstenite::Message;

/// A boot failure — the gateway could not reach control, was shipped no/invalid config, or could not bind
/// its HTTP port. The binary reports it and exits non-zero so the harness driver sees the boot failed.
#[derive(Debug)]
pub enum BootError {
    /// The TCP dial or WebSocket handshake to the control server failed.
    ControlDial(String),
    /// The control link closed before shipping a config frame.
    NoConfig,
    /// The first frame control shipped was not a decodable [`ControlConfig`].
    BadConfig,
    /// Binding or serving the HTTP listener failed.
    Listen(std::io::Error),
}

impl std::fmt::Display for BootError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BootError::ControlDial(e) => write!(f, "could not dial control server: {e}"),
            BootError::NoConfig => write!(f, "control link closed before shipping a ControlConfig"),
            BootError::BadConfig => write!(f, "control shipped an undecodable ControlConfig frame"),
            BootError::Listen(e) => write!(f, "HTTP listener error: {e}"),
        }
    }
}

impl std::error::Error for BootError {}

/// Boot the gateway: dial `control_addr`, apply the `ControlConfig` control ships on connect, and serve
/// HTTP on `listen_addr` until the listener errors. `listen_addr`/`control_addr` are `host:port` strings
/// (a `:0` listen port binds an ephemeral port — the bound address is printed to stderr for the harness).
///
/// # Errors
/// Returns [`BootError`] if control is unreachable, ships no/invalid config, or the HTTP port cannot be bound.
pub async fn run(listen_addr: &str, control_addr: &str) -> Result<(), BootError> {
    // 1. Dial the persistent control link and apply the config it ships on connect.
    let (config, control_ws) = dial_control(control_addr).await?;
    let _cas = HttpBlobStore::new(config.cas_url.as_str())
        .with_read_credential(String::from_utf8_lossy(&config.cas_credential).into_owned());

    // Keep the control link OPEN for the gateway's whole life (§3): a persistent bidirectional bus. This
    // stub drains it (consuming keepalives + any pushed frames) so the connection stays established; folding
    // pushed config/root-router updates (live-swap) and demuxing ControlDown to sessions are later slices.
    tokio::spawn(async move {
        let mut ws = control_ws;
        while let Some(Ok(_frame)) = ws.next().await {}
    });

    // 2. Bind the HTTP edge and announce the bound address (the driver passes :0 and reads the real port).
    let listener = TcpListener::bind(listen_addr)
        .await
        .map_err(BootError::Listen)?;
    let bound = listener.local_addr().map_err(BootError::Listen)?;
    eprintln!("gateway: listen={bound} control={control_addr}");

    // 3. Serve. Routing (drive the root-router program) is the next slice; the stub floors every request.
    serve(listener).await
}

/// The control link, once dialed and configured: the applied [`ControlConfig`] plus the still-open ws.
/// `client_async` over a raw `TcpStream` (no TLS/connect feature) yields `WebSocketStream<TcpStream>`.
type ControlLink = tokio_tungstenite::WebSocketStream<TcpStream>;

/// Dial the control server over a WebSocket and read the `ControlConfig` it ships as its first data frame
/// on connect (§3). Returns the decoded config and the still-open link (kept alive for the gateway's life).
async fn dial_control(control_addr: &str) -> Result<(ControlConfig, ControlLink), BootError> {
    let stream = TcpStream::connect(control_addr)
        .await
        .map_err(|e| BootError::ControlDial(e.to_string()))?;
    let (mut ws, _resp) = tokio_tungstenite::client_async(format!("ws://{control_addr}/"), stream)
        .await
        .map_err(|e| BootError::ControlDial(e.to_string()))?;
    let codec = control_frame_codec();
    // The config is the first DATA frame control pushes on connect; skip protocol control frames.
    loop {
        match ws.next().await {
            Some(Ok(Message::Binary(bytes))) => {
                return Ok((decode_boot_config(&codec, &bytes)?, ws));
            }
            Some(Ok(Message::Text(text))) => {
                return Ok((decode_boot_config(&codec, text.as_bytes())?, ws));
            }
            Some(Ok(_)) => continue, // ping/pong/other control frame — keep waiting for the config
            _ => return Err(BootError::NoConfig), // stream error or closed before any data frame
        }
    }
}

/// The control-link frame codec, tagging each frame by its CANONICAL COMPUTED contract-id — the descriptor
/// ids of the `cdz-platform.control.{config,up,down}` userspace contracts, reachable now that
/// `cdz_platform::contracts` is public. Both the gateway (here) and the control server derive the SAME ids
/// from these contracts, so a tagged frame round-trips without markers.
fn control_frame_codec() -> FrameCodec {
    fn id(c: cdz_platform::Contract) -> Bytes {
        Bytes::copy_from_slice(c.id().hash().as_bytes())
    }
    FrameCodec::new(
        id(cdz_platform::contracts::control_config::contract()),
        id(cdz_platform::contracts::control_up::contract()),
        id(cdz_platform::contracts::control_down::contract()),
    )
}

/// Decode the boot [`ControlConfig`] from control's first frame, accepting BOTH the tagged [`ControlFrame`]
/// envelope (the computed-id frame-dispatch protocol) AND a bare `ControlConfig` (the pre-flip form). This
/// is the rolling-upgrade step: the gateway handles both while the control server's send-side flips from
/// bare to enveloped, so neither side breaks the other at the flip. Once control always sends enveloped,
/// the bare fallback is dropped.
fn decode_boot_config(codec: &FrameCodec, bytes: &[u8]) -> Result<ControlConfig, BootError> {
    if let Some(ControlFrame::Config(config)) = codec.decode(bytes) {
        return Ok(config);
    }
    decode_control_config(bytes).ok_or(BootError::BadConfig)
}

/// The HTTP accept loop: one hyper HTTP/1 connection per socket, each request served the boot floor.
async fn serve(listener: TcpListener) -> Result<(), BootError> {
    loop {
        let (stream, _peer) = listener.accept().await.map_err(BootError::Listen)?;
        let io = TokioIo::new(stream);
        tokio::spawn(async move {
            // A connection-level error is the client's business, not the edge's — drop it and keep serving.
            let _ = hyper::server::conn::http1::Builder::new()
                .serve_connection(
                    io,
                    service_fn(|_req| async { Ok::<_, Infallible>(boot_floor()) }),
                )
                .await;
        });
    }
}

/// The stub's response until routing is wired: a `200` announcing the gateway booted from control. Replaced
/// by driving the control-shipped root router (route → dispatch → handler response, with 404/500/504 floors)
/// in the next slice.
fn boot_floor() -> Response<Full<Bytes>> {
    Response::builder()
        .status(StatusCode::OK)
        .body(Full::new(Bytes::from_static(
            b"cdz-http-gateway: booted from control (routing not yet wired)\n",
        )))
        .expect("static boot-floor response is valid")
}

#[cfg(test)]
mod tests {
    use super::*;
    use cdz_http_protocol::encode_control_config;
    use cdz_platform::Str;

    fn sample_config() -> ControlConfig {
        ControlConfig {
            cas_url: Str::from("http://cas.internal:9000"),
            cas_credential: Bytes::from_static(b"bearer-abc"),
            root_router: Bytes::from_static(b"a-33-byte-program-hash-goes-here."),
        }
    }

    #[test]
    fn decode_boot_config_accepts_both_the_tagged_envelope_and_the_bare_config() {
        let codec = control_frame_codec();
        let config = sample_config();

        // The bare pre-flip form (control server's current send-side): `encode_control_config`.
        let bare = encode_control_config(&config);
        assert_eq!(decode_boot_config(&codec, &bare).unwrap(), config);

        // The tagged form (control server's post-flip send-side): a `ControlFrame::Config` envelope.
        let enveloped = codec.encode(&ControlFrame::Config(config.clone()));
        assert_eq!(decode_boot_config(&codec, &enveloped).unwrap(), config);

        // Neither → a boot config error, not a panic.
        assert!(matches!(
            decode_boot_config(&codec, b"not a config"),
            Err(BootError::BadConfig)
        ));
    }
}
