//! Boot-from-control (`DESIGN-http-outpost-drive-contract.md` §3/§4) — the dumb gateway's startup path.
//!
//! The gateway is launched knowing ONLY two things: where to serve HTTP and where the control server is.
//! **Everything else — the CAS url + credential, the root-router `ProgramHash`, and any future knob — ships
//! from the control server** in the [`ControlConfig`] it pushes over the persistent link, so the gateway is
//! reconfigured ON THE FLY (a pushed config live-swaps the applied one; nothing but the two addresses is
//! baked into the process). [`run`] binds the HTTP listener FIRST so the edge is up immediately, then dials
//! the control link in the background and applies each config it ships.
//!
//! **Readiness (operator directive):** until control has shipped a config, the gateway has no program to run,
//! so every request is answered a specific **`503 Service Unavailable`** (`waiting for control` — the edge is
//! up but not yet configured), NOT a misleading `200`. Once a config arrives the gateway is ready; a later
//! pushed config live-swaps it without dropping readiness or restarting.
//!
//! **Serving (once configured):** each request drives the control-shipped **root-router program** — spawn it
//! from the CAS by hash, deliver the request as a bare `http-request` value (§4), drive it over a fresh
//! mailbox through [`crate::drive`] + the [`crate::resolver`] effect resolver (§1/§2), and turn its terminal
//! `Break` into the HTTP response (§6): `http.response` ⇒ the answer with an INLINE body, `http.response-cas`
//! ⇒ the answer with a body FETCHED from the CAS by hash (the `CasRef` half of §6), `http.deny` ⇒ the handler's
//! `status` + `reason`, else ⇒ `500`.
//!
//! **Control link (§3):** the one persistent ws is a bidirectional bus, DEMUXED by contract-id — `Config`
//! (re)configures (live-swap), `Down` is a session-addressed message routed to the running handler (the
//! session registry that closes that loop, plus `ws.send`, is the remaining follow-on slice).

use crate::drive::drive;
use crate::resolver::{ControlCtx, GatewayResolver};
use crate::session::{ControlSink, Sessions};
use bytes::Bytes;
use cdz_http_protocol::{
    ControlConfig, ControlFrame, ControlUp, FrameCodec, Header, RequestContext,
    decode_control_config, value,
};
use cdz_platform::{
    BlobStore, ContractId, Delivered, Hash, HostId, Message as ReducerMessage, Origin, ProgramHash,
    ProgramStore, ReducerId, ReducerKind, SpawnContext, Str, TokioRuntime,
};
use futures_util::{SinkExt, StreamExt};
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use std::convert::Infallible;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};
use std::time::Duration;
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::tungstenite::Message;

/// A process-wide counter minting a distinct session id per driven HTTP request, so a `control.send`'s
/// `ControlUp`/`ControlDown` can be attributed to the request that emitted it (routing back is by
/// `correlation`, but the session id lets the control server distinguish concurrent requests).
static NEXT_SESSION: AtomicU64 = AtomicU64::new(0);

/// How deep a `http.dispatch` chain (router → handler → sub-handler → …) may recurse before a dispatch is
/// answered "missing" — a fixed bound so a cyclic/runaway dispatch cannot recurse forever. Not control-shipped
/// (a safety limit, not policy); the resolver decrements it per hop.
const DISPATCH_DEPTH: usize = 16;

/// The maximum request body the edge buffers before routing (design §8 #5): a body larger than this is
/// answered `413 Payload Too Large` WITHOUT driving a program. A baked SAFETY ceiling (like [`DISPATCH_DEPTH`]),
/// not control-shipped policy — generous by default; it can move to `ControlConfig` if the operator wants it
/// tunable (a contract-id change then). 16 MiB.
const MAX_REQUEST_BODY: usize = 16 << 20;

/// Why marshalling an incoming request failed — selects the client-error floor.
enum RequestReadError {
    /// Unsupported method, or an unreadable/malformed body ⇒ `400`.
    BadRequest,
    /// The body exceeds [`MAX_REQUEST_BODY`] (by declared `Content-Length` or by actual bytes) ⇒ `413`,
    /// answered BEFORE the program is driven (§8 #5).
    TooLarge,
}

/// Whether a declared `Content-Length` exceeds [`MAX_REQUEST_BODY`]. `None` (absent/unparseable length) is not
/// over the ceiling by declaration — the read-side `Limited` still bounds the actual bytes. Pure, so the
/// ceiling decision is unit-testable without constructing a hyper body.
fn content_length_exceeds_ceiling(len: Option<u64>) -> bool {
    len.is_some_and(|n| n > MAX_REQUEST_BODY as u64)
}

/// The canonical contract-ids (§5) the gateway routes by — the descriptor-derived ids of the platform's
/// `http.*` contracts (same derivation the guest + control server use, never ad-hoc markers). Computed once
/// when a config is applied. Only the `host` build resolves a config to a ready state, so in the light spine
/// build it is constructed nowhere (its fields are still read on the drive path).
#[cfg_attr(not(feature = "host"), allow(dead_code))]
#[derive(Clone)]
struct Ids {
    /// `http.dispatch` — a root router's "spawn this subprogram and hand it this input" effect.
    dispatch: ContractId,
    /// `http.request` — the schema the gateway delivers an incoming request under (the driven program's
    /// opening `on_message`).
    request: ContractId,
    /// `http.response` — the terminal `Break` schema a program answers a request with (INLINE body, §6).
    response: ContractId,
    /// `http.response-cas` — the terminal `Break` schema a program answers with when the body lives in the
    /// CAS (§6 `CasRef`): the reason carries a blob hash the edge fetches + serves.
    response_cas: ContractId,
    /// `http.deny` — the terminal `Break` schema a program rejects a request with.
    deny: ContractId,
}

/// The gateway's applied drive context — everything control's config resolves to (§1/§4/§5). Held in a
/// live-swappable [`SharedState`] slot: `None` until control configures us (⇒ `503`), then the drive context
/// built from the latest config control pushed. Cheaply cloneable (the store is an `Arc`), so the serve loop
/// snapshots it per request without holding the lock across the drive.
#[derive(Clone)]
pub struct GatewayState {
    /// The wasm-backed program store over the control-supplied CAS: fetches + instantiates a program by hash.
    store: Arc<dyn ProgramStore>,
    /// A read-capable handle to the same control-supplied CAS, for resolving a `CasRef` response body (§6):
    /// a handler answers `http.response-cas` with a blob hash and the edge fetches the bytes here.
    cas: Arc<dyn BlobStore>,
    /// The control-shipped root-router `ProgramHash` — the program driven for every request (§4).
    root_router: ProgramHash,
    /// The canonical contract-ids the gateway routes effects + terminal breaks by (§5).
    ids: Ids,
    /// The write half of the live control link a handler's `control.send` is forwarded UP through (§3). Tied
    /// to the connection this config arrived on; a redial rebuilds the state with a fresh sink.
    control_sink: ControlSink,
    /// The pending-`control.send` registry: routes the control server's `ControlDown` response back into the
    /// awaiting reducer (shared with the control-link read task).
    sessions: Sessions,
    /// The `control.send` effect's canonical contract-id (§5) — a request on this id is forwarded UP.
    control_send: ContractId,
}

/// The gateway's readiness slot: `None` = not yet configured (serve `503`), `Some` = the config control last
/// pushed. Shared between the control-link task (which swaps it on every pushed config — live reconfig) and
/// the HTTP serve loop (which reads it per request). A plain `RwLock` since a read is a cheap clone-out.
type SharedState = Arc<RwLock<Option<GatewayState>>>;

/// A boot failure — the gateway could not reach control, was shipped no/invalid config, or could not bind
/// its HTTP port. The binary reports it and exits non-zero so the harness driver sees the boot failed.
#[derive(Debug)]
pub enum BootError {
    /// The TCP dial or WebSocket handshake to the control server failed.
    ControlDial(String),
    /// Binding or serving the HTTP listener failed.
    Listen(std::io::Error),
}

impl std::fmt::Display for BootError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BootError::ControlDial(e) => write!(f, "could not dial control server: {e}"),
            BootError::Listen(e) => write!(f, "HTTP listener error: {e}"),
        }
    }
}

impl std::error::Error for BootError {}

/// Boot the gateway: bind HTTP on `listen_addr` immediately, dial `control_addr` in the background, apply
/// every `ControlConfig` control ships (live reconfig), and serve until the listener errors.
/// `listen_addr`/`control_addr` are `host:port` strings (a `:0` listen port binds an ephemeral port — the
/// bound address is printed to stderr for the harness driver). Until control ships a config, requests are
/// answered `503` (the edge is up but not yet configured).
///
/// # Errors
/// Returns [`BootError::Listen`] only if the HTTP port cannot be bound. Control being momentarily
/// unreachable is NOT fatal — the gateway serves `503` and the control task retries until control answers.
pub async fn run(listen_addr: &str, control_addr: &str) -> Result<(), BootError> {
    // 1. Bind the HTTP edge FIRST so the gateway is reachable immediately, and announce the bound address
    //    (the driver passes :0 and reads the real port). Requests before control configures us get `503`.
    let listener = TcpListener::bind(listen_addr)
        .await
        .map_err(BootError::Listen)?;
    let bound = listener.local_addr().map_err(BootError::Listen)?;
    eprintln!("gateway: listen={bound} control={control_addr}");

    // 2. The readiness slot: `None` until control ships a config. The control task fills + live-swaps it; the
    //    serve loop reads it per request. Everything but the two launch addresses lives HERE, from control.
    let state: SharedState = Arc::new(RwLock::new(None));

    // The pending-`control.send` registry — shared for the process's life (survives redials) between the
    // control-link read task (which routes `ControlDown` responses) and the driven reducers (which register).
    let sessions = Sessions::new();

    // 3. Dial control in the background and keep the link open for the gateway's whole life (§3), applying
    //    each config it ships. Retries on dial failure/link drop so a slow-to-start control just delays
    //    readiness (503 meanwhile) rather than killing the gateway.
    tokio::spawn(control_link(
        control_addr.to_string(),
        Arc::clone(&state),
        sessions,
    ));

    // 4. Serve immediately: `503` until ready, then drive the control-shipped root router per request.
    serve(listener, state).await
}

/// The control link, once dialed: the still-open ws. `client_async` over a raw `TcpStream` (no TLS/connect
/// feature) yields `WebSocketStream<TcpStream>`.
type ControlLink = tokio_tungstenite::WebSocketStream<TcpStream>;

/// The persistent control-link task (§3): dial control, then read every frame it pushes, applying each
/// [`ControlConfig`] into `state` — the first makes the gateway ready, a later one live-swaps the applied
/// config (on-the-fly reconfig). On a dial failure or a dropped link it backs off and redials, so the
/// gateway keeps serving `503` while control is unreachable rather than exiting. Runs for the process's life.
async fn control_link(control_addr: String, state: SharedState, sessions: Sessions) {
    let codec = control_frame_codec();
    // Consecutive dial failures since the last successful dial — drives the redial backoff so a
    // persistently-unreachable control is retried with exponential (capped) spacing rather than a flat
    // 250 ms busy-spin. Reset to 0 the moment a dial succeeds, so a transient link DROP (control was just
    // reachable) redials promptly.
    let mut failures: u32 = 0;
    loop {
        match dial_control(&control_addr).await {
            Ok(ws) => {
                failures = 0;
                // Split the one ws into a read half (frames DOWN) and a write half (frames UP). A
                // `control.send` reaches the write half via an unbounded mpsc: the resolver pushes a
                // `ControlUp` into `up_tx` (fire-and-forget), and the writer task below drains it to the ws —
                // so a handler's effect never blocks the drive loop on the socket.
                let (mut write, mut read) = ws.split();
                let (up_tx, mut up_rx) = tokio::sync::mpsc::unbounded_channel::<ControlUp>();
                let writer = tokio::spawn(async move {
                    let codec = control_frame_codec();
                    while let Some(up) = up_rx.recv().await {
                        let frame = codec.encode(&ControlFrame::Up(up));
                        if write.send(Message::Binary(frame.to_vec())).await.is_err() {
                            break; // link write half is gone — stop; the read loop will redial.
                        }
                    }
                });

                // Read every frame control pushes and DEMUX IT BY CONTRACT ID (the computed-id frame
                // dispatch): the same link carries config, session-addressed messages, and (unexpectedly) up
                // frames. A closed or errored link breaks out to redial.
                while let Some(Ok(msg)) = read.next().await {
                    let Some(bytes) = frame_bytes(&msg) else {
                        continue; // ping/pong/close — no payload to demux
                    };
                    match classify_frame(&codec, bytes) {
                        Some(ControlFrame::Config(config)) => {
                            // Build the drive context from the config (CAS store + root-router hash +
                            // canonical ids + THIS link's write half + the session registry) and live-swap it
                            // in: the first config makes us ready; a later one reconfigures on the fly (only
                            // the two launch addresses are baked in). A config we cannot apply (bad hash, or no
                            // wasm engine in the light spine build) leaves us `None` ⇒ still `503`.
                            match ready_state(&config, up_tx.clone(), sessions.clone()) {
                                Some(next) => {
                                    if let Ok(mut slot) = state.write() {
                                        *slot = Some(next);
                                    }
                                }
                                None => eprintln!(
                                    "gateway: config not applicable; staying unconfigured"
                                ),
                            }
                        }
                        Some(ControlFrame::Down(down)) => {
                            // A control-server response addressed by `correlation` to the handler that emitted
                            // a `control.send` — fold its payload back into that reducer's `on_response` (§3).
                            // An unmatched correlation (already answered, or a push with no waiter) is dropped.
                            sessions.route_response(&down.correlation, down.payload);
                        }
                        // The gateway SENDS `Up` frames (a handler's `control.send`); receiving one down the
                        // link is unexpected. An unknown/undecodable frame is likewise ignored, not fatal.
                        Some(ControlFrame::Up(_)) | None => {}
                    }
                }
                // Link dropped — stop the writer (its ws write half is dead) before redialing.
                writer.abort();
            }
            Err(e) => {
                // Control unreachable or handshake failed — stay up (503) and retry.
                failures = failures.saturating_add(1);
                eprintln!("gateway: control link down ({e}); retrying");
            }
        }
        // Back off before redialing so a persistently-unreachable control does not busy-spin: 250 ms
        // doubling per consecutive failure, capped, and reset to the 250 ms floor on any successful dial.
        tokio::time::sleep(redial_backoff(failures)).await;
    }
}

/// The redial backoff for consecutive control-link dial `failures` (0 = redial after a successful dial, e.g.
/// a transient drop): a 250 ms floor doubling per failure, capped at 5 s so a long control outage is retried
/// steadily (~every 5 s) without busy-spinning. Pure so the schedule is unit-testable.
fn redial_backoff(failures: u32) -> Duration {
    const FLOOR_MS: u64 = 250;
    const CAP_MS: u64 = 5_000;
    // Shift the floor left by `failures`, saturating: 250, 500, 1000, 2000, 4000, then the 5 s cap. `>= 20`
    // would overflow the u64 shift, so clamp the exponent first; the cap makes anything past ~5 identical.
    let ms = FLOOR_MS
        .checked_shl(failures.min(20))
        .unwrap_or(CAP_MS)
        .min(CAP_MS);
    Duration::from_millis(ms)
}

/// Dial the control server over a WebSocket (§3), returning the still-open link. Frames are read by the
/// caller ([`control_link`]).
async fn dial_control(control_addr: &str) -> Result<ControlLink, BootError> {
    let stream = TcpStream::connect(control_addr)
        .await
        .map_err(|e| BootError::ControlDial(e.to_string()))?;
    let (ws, _resp) = tokio_tungstenite::client_async(format!("ws://{control_addr}/"), stream)
        .await
        .map_err(|e| BootError::ControlDial(e.to_string()))?;
    Ok(ws)
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

/// The payload bytes of a ws message that carries a control frame — a binary or text data frame; `None` for
/// a ping/pong/close (nothing to demux).
fn frame_bytes(msg: &Message) -> Option<&[u8]> {
    match msg {
        Message::Binary(b) => Some(b.as_ref()),
        Message::Text(t) => Some(t.as_bytes()),
        _ => None,
    }
}

/// Classify a control frame by its CANONICAL COMPUTED contract-id (§5): the [`FrameCodec`] matches the
/// frame's id tag against the `control.{config,up,down}` descriptor ids and returns the typed [`ControlFrame`]
/// — so the read loop DEMUXES config vs a session-addressed message (§3) rather than assuming every frame is
/// a config. Falls back to a bare (untagged) `ControlConfig` for a pre-flip control server (the rolling-
/// upgrade step; dropped once control always sends enveloped). `None` for an undecodable/unknown frame.
fn classify_frame(codec: &FrameCodec, bytes: &[u8]) -> Option<ControlFrame> {
    codec
        .decode(bytes)
        .or_else(|| decode_control_config(bytes).map(ControlFrame::Config))
}

/// Build the drive context (§1/§3/§4/§5) from a control-shipped config: the wasm-backed program store over
/// the config's CAS, the root-router hash, the canonical contract-ids, and the control back-channel (the
/// live link's write half `sink` + the shared `sessions` registry). `None` if the config is unusable — a
/// malformed root-router hash, or (in the light non-`host` spine build) no wasm engine to run programs.
#[cfg(feature = "host")]
fn ready_state(
    config: &ControlConfig,
    sink: ControlSink,
    sessions: Sessions,
) -> Option<GatewayState> {
    let root_router = ProgramHash::try_from(config.root_router.as_ref()).ok()?;
    let (store, cas) = crate::wasm::build_store(config.cas_url.as_str(), &config.cas_credential)
        .map_err(|e| eprintln!("gateway: wasm program store init failed: {e}"))
        .ok()?;
    let ids = canonical_ids();
    let control_send = cdz_platform::contracts::control_send::contract().id();
    Some(GatewayState {
        store,
        cas,
        root_router,
        ids,
        control_sink: sink,
        sessions,
        control_send,
    })
}

/// The light spine (no `host` feature) has no wasm engine, so it can never drive a program — it stays
/// unconfigured (⇒ `503`). Only the real gateway binary (which enables `host`) becomes ready.
#[cfg(not(feature = "host"))]
fn ready_state(
    _config: &ControlConfig,
    _sink: ControlSink,
    _sessions: Sessions,
) -> Option<GatewayState> {
    None
}

/// The canonical contract-ids (§5): the descriptor-derived ids of the platform's `http.*` contracts — the
/// SAME derivation the guest programs + control server use, so a routed effect/break matches without markers.
/// Only the `host` build resolves a config to a ready state, so this is `host`-gated.
#[cfg(feature = "host")]
fn canonical_ids() -> Ids {
    fn id(c: cdz_platform::Contract) -> ContractId {
        c.id()
    }
    Ids {
        dispatch: id(cdz_platform::contracts::http_dispatch::contract()),
        request: id(cdz_platform::contracts::http_request::contract()),
        response: id(cdz_platform::contracts::http_response::contract()),
        response_cas: id(cdz_platform::contracts::http_response_cas::contract()),
        deny: id(cdz_platform::contracts::http_deny::contract()),
    }
}

/// The HTTP accept loop: one hyper HTTP/1 connection per socket. Each request is answered from a snapshot of
/// the current readiness `state` — `503` until control configures us, else the control-shipped root router's
/// response.
async fn serve(listener: TcpListener, state: SharedState) -> Result<(), BootError> {
    loop {
        let (stream, _peer) = listener.accept().await.map_err(BootError::Listen)?;
        let io = TokioIo::new(stream);
        let state = Arc::clone(&state);
        tokio::spawn(async move {
            // A connection-level error is the client's business, not the edge's — drop it and keep serving.
            let _ = hyper::server::conn::http1::Builder::new()
                .serve_connection(
                    io,
                    service_fn(move |req: Request<Incoming>| {
                        // Snapshot readiness at request time (cheap clone-out; guard dropped before the drive).
                        let ready = state.read().ok().and_then(|s| s.clone());
                        async move { Ok::<_, Infallible>(handle(ready, req).await) }
                    }),
                )
                .await;
        });
    }
}

/// Answer one request: `503` until control configures us (the operator-requested "waiting for control"
/// status, never a misleading `200`), else drive the control-shipped root router (§1/§4) and turn its
/// terminal `Break` into the HTTP response (§6).
async fn handle(state: Option<GatewayState>, req: Request<Incoming>) -> Response<Full<Bytes>> {
    let Some(gw) = state else {
        return status(
            StatusCode::SERVICE_UNAVAILABLE,
            b"cdz-http-gateway: waiting for control (no program configured yet)\n",
        );
    };
    drive_request(&gw, req).await
}

/// Drive the control-shipped root router for one HTTP request (§1): marshal the request into a bare
/// `http-request` value, spawn the router from the CAS, drive it over a fresh mailbox with the gateway effect
/// resolver (fire-and-forget effects, §2), and turn its terminal `Break` into the response — `http.response`
/// ⇒ the answer (§6), `http.deny` ⇒ `403`, anything else / no terminal ⇒ `500`.
async fn drive_request(gw: &GatewayState, req: Request<Incoming>) -> Response<Full<Bytes>> {
    let (payload, request) = match encode_request(req).await {
        Ok(pair) => pair,
        // §8 #5: an oversized body is floored `413` BEFORE any program is driven.
        Err(RequestReadError::TooLarge) => {
            return status(
                StatusCode::PAYLOAD_TOO_LARGE,
                b"cdz-http-gateway: request body exceeds the 16 MiB ceiling\n",
            );
        }
        Err(RequestReadError::BadRequest) => {
            return status(
                StatusCode::BAD_REQUEST,
                b"cdz-http-gateway: could not read request\n",
            );
        }
    };
    let ctx = SpawnContext {
        id: ReducerId::of(gw.root_router.hash().as_bytes()),
        kind: ReducerKind::Ordinary,
        limits: None,
    };
    let Some(reducer) = gw.store.spawn(gw.root_router, ctx).await else {
        // Make the spawn failure ACTIONABLE rather than an opaque 502: probe whether the component is even in
        // the CAS, so the log distinguishes a fetch/seeding gap from an instantiate/dependency failure.
        let present = gw.store.contains(gw.root_router).await;
        let hash: String = gw
            .root_router
            .hash()
            .as_bytes()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        if present {
            eprintln!(
                "gateway: root router {hash} is in the CAS but FAILED TO INSTANTIATE — likely a \
                 dependency component (e.g. the value-heap runtime a Cadenza guest imports) is not \
                 seeded in the CAS, or a linker/world mismatch"
            );
        } else {
            eprintln!(
                "gateway: root router {hash} is NOT in the CAS — the fetch failed; check the config's \
                 cas_url/credential and that the program + its full dependency closure are seeded"
            );
        }
        return status(
            StatusCode::BAD_GATEWAY,
            b"cdz-http-gateway: root router program unavailable\n",
        );
    };
    let opening = Delivered::Message(ReducerMessage {
        id: gw.ids.request,
        payload,
        from: edge_origin(),
        continuation_token: Bytes::new(),
    });
    // A distinct session id per driven request, so a `control.send`'s ControlUp/ControlDown is attributable
    // to this request (the response routes back by `correlation`; the session id distinguishes concurrency).
    let session = Bytes::from(
        NEXT_SESSION
            .fetch_add(1, Ordering::Relaxed)
            .to_be_bytes()
            .to_vec(),
    );
    // Drive with the control back-channel wired (§3): a handler's `control.send` is forwarded UP and its
    // response folds back via the shared session registry, stamped with this request's context + session.
    let resolver = GatewayResolver::new_with_control(
        Arc::clone(&gw.store),
        gw.ids.dispatch,
        gw.ids.request,
        DISPATCH_DEPTH,
        ControlCtx {
            sink: gw.control_sink.clone(),
            sessions: gw.sessions.clone(),
            control_send: gw.control_send,
            session,
            request: Arc::new(request),
            program: gw.root_router,
        },
    );
    let out = drive::<TokioRuntime>(reducer, opening, move |r, tx, scope| {
        Arc::clone(&resolver).carry::<TokioRuntime>(r, tx, scope)
    })
    .await;
    match out {
        Some((schema, reason)) if schema == gw.ids.response => decode_response(&reason),
        Some((schema, reason)) if schema == gw.ids.response_cas => {
            decode_response_cas(&gw.cas, &reason).await
        }
        Some((schema, reason)) if schema == gw.ids.deny => decode_deny(&reason),
        // The program closed without a terminal http-response/deny, or a fold panicked → a gateway error.
        _ => status(
            StatusCode::INTERNAL_SERVER_ERROR,
            b"cdz-http-gateway: no response from program\n",
        ),
    }
}

/// The synthetic edge [`Origin`] a driven request is delivered from — the gateway edge, so the program sees
/// an unforgeable provenance envelope (§0) even for the opening request.
fn edge_origin() -> Origin {
    Origin {
        reducer: ReducerId::of(b"cdz-http-gateway.edge"),
        host: HostId::of(b"cdz-http-gateway"),
    }
}

/// Marshal an incoming HTTP request into (the bare `http-request` value the driven program folds as its
/// opening event, §4) plus (the [`RequestContext`] a `control.send` carries UP so the control server can
/// route without re-parsing the payload, §3). Reads the whole body (buffered v0), bounded by the
/// [`MAX_REQUEST_BODY`] ceiling (§8 #5). `Err(TooLarge)` if the body exceeds it (⇒ `413` before routing),
/// `Err(BadRequest)` on an unsupported method or a body-read failure (⇒ `400`).
async fn encode_request(
    req: Request<Incoming>,
) -> Result<(Bytes, RequestContext), RequestReadError> {
    let (parts, mut body) = req.into_parts();
    let method = method_value_kind(&parts.method).ok_or(RequestReadError::BadRequest)?;
    // §8 #5: whether the DECLARED Content-Length already puts the body over the ceiling. We do NOT reject
    // early on it — we still drain the body below so the client finishes uploading and receives a
    // CLIENT-VISIBLE 413 (returning + closing the socket mid-upload surfaces as a connection reset, not the
    // 413 — the 413-while-uploading race).
    let declared_len = parts
        .headers
        .get(hyper::header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse::<u64>().ok());
    let declared_over = content_length_exceeds_ceiling(declared_len);
    let path = parts.uri.path().to_string();
    let query = parts.uri.query().unwrap_or("").to_string();
    let headers: Vec<(String, String)> = parts
        .headers
        .iter()
        .filter_map(|(n, v)| {
            v.to_str()
                .ok()
                .map(|v| (n.as_str().to_string(), v.to_string()))
        })
        .collect();
    // Read the body, buffering at most `MAX_REQUEST_BODY`; if the declared length OR the actual bytes exceed
    // the ceiling, keep DRAINING (discard) to EOF so the client's upload completes and it receives a
    // client-visible `413` (not a mid-upload reset), then flag `TooLarge`. Memory stays constant once over.
    let mut buf: Vec<u8> = Vec::new();
    let mut over = declared_over;
    while let Some(frame) = body.frame().await {
        let frame = frame.map_err(|_| RequestReadError::BadRequest)?;
        if let Ok(chunk) = frame.into_data() {
            if over || buf.len() + chunk.len() > MAX_REQUEST_BODY {
                over = true;
                buf = Vec::new();
            } else {
                buf.extend_from_slice(&chunk);
            }
        }
    }
    if over {
        return Err(RequestReadError::TooLarge);
    }
    let body = Bytes::from(buf);
    let payload = encode_request_value(method, &path, &query, &headers, &body);
    let context = RequestContext {
        method: Str::from(parts.method.as_str()),
        path: Str::from(path.as_str()),
        headers: headers
            .iter()
            .map(|(name, value)| Header {
                name: Str::from(name.as_str()),
                value: Str::from(value.as_str()),
            })
            .collect(),
    };
    Ok((payload, context))
}

/// The `http.request` `Method` variant for a hyper [`Method`], or `None` for one the contract has no case for
/// (e.g. `CONNECT`/`TRACE`/an extension method) — the gateway answers such a request `400` rather than guess.
fn method_value_kind(method: &Method) -> Option<&'static str> {
    Some(match *method {
        Method::GET => "Get",
        Method::POST => "Post",
        Method::PUT => "Put",
        Method::DELETE => "Delete",
        Method::PATCH => "Patch",
        Method::HEAD => "Head",
        Method::OPTIONS => "Options",
        _ => return None,
    })
}

/// Build the `Request.Request` value (§4) from its parts, using the platform's canonical `http-request`
/// contract builders so it type-ascribes against the schema the driven program decodes. The ROOT ascription
/// is RETAINED (`finish`, not `finish_value`): the GUEST's compiled Cadenza `Value.decode(Request)` is NOT
/// ascription-invariant — it returns `None` on an ascription-free root (→ a 400 "undecodable http-request"),
/// which #8770 hit and reverted here (the Rust-side decode-invariance of #8758 does not cover the guest's
/// compiled decode). Until the guest decode is made ascription-invariant (v-value-codec / compiler), this
/// terminal ascription stays. (The control-link encoders CAN drop it — their decoders unascribe.)
fn encode_request_value(
    method: &'static str,
    path: &str,
    query: &str,
    headers: &[(String, String)],
    body: &[u8],
) -> Bytes {
    use cdz_platform::contracts::http_request as reqc;
    let mut b = value::ValueBuilder::new();
    // A nullary `Method` variant is `(Ctor unit)`.
    let unit = value::unit(&mut b);
    let method = value::bare_ctor(&mut b, method, vec![unit]);
    let path = value::str_leaf(&mut b, path);
    let query = value::str_leaf(&mut b, query);
    // Each list element must carry its own `(: <record> Header)` ascription: the guest's `Value.encode`
    // ascribes a single-constructor RECORD value (a `Header` newtype) so `Value.decode` can disambiguate the
    // elided `#record` back to `Header`, and its decode of `List(Header)` REQUIRES that per-element ascription.
    // `reqc::header_header` builds the bare record (single-ctor elided, no ascription — correct for a value
    // whose type is fixed by an enclosing ascription, e.g. a root or a same-typed field), so a list element
    // needs the wrap. Without it, any request WITH headers fails to decode in the guest (an empty header list
    // is unaffected — hence it hid until a header-bearing request was driven end to end).
    let header_values: Vec<value::ValueId> = headers
        .iter()
        .map(|(name, val)| {
            let name = value::str_leaf(&mut b, name);
            let value = value::str_leaf(&mut b, val);
            let header = reqc::header_header(&mut b, reqc::HeaderHeader { name, value });
            value::ascribe(&mut b, header, "Header")
        })
        .collect();
    let headers = value::list_value(&mut b, header_values);
    let body = value::bytes_leaf(&mut b, body);
    let request = reqc::request_request(
        &mut b,
        reqc::RequestRequest {
            method,
            path,
            query,
            headers,
            body,
        },
    );
    value::finish(b, request, "Request")
}

/// Turn a program's terminal `http.response` `Break` reason — a `Response.Response` value (§6) — into the
/// HTTP response: `status` (`Int64`) + `headers` (`List(Header)`) + `body` (`Bytes`, inline v0). A malformed
/// value or an unrepresentable status/header ⇒ a `500` (the program answered nonsense).
fn decode_response(reason: &[u8]) -> Response<Full<Bytes>> {
    let Some(arenas) = value::decode(reason) else {
        return status(
            StatusCode::INTERNAL_SERVER_ERROR,
            b"cdz-http-gateway: undecodable program response\n",
        );
    };
    let root = arenas.root;
    let code = value::record_field(&arenas, root, "status")
        .and_then(|f| value::read_uint(&arenas, f))
        .and_then(|u| u16::try_from(u).ok())
        .and_then(|u| StatusCode::from_u16(u).ok())
        .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    let body = value::record_field(&arenas, root, "body")
        .and_then(|f| value::read_bytes(&arenas, f))
        .unwrap_or_default();
    let mut builder = Response::builder().status(code);
    if let Some(headers) =
        value::record_field(&arenas, root, "headers").and_then(|f| value::read_list(&arenas, f))
    {
        for &h in headers {
            if let (Some(name), Some(val)) = (
                value::record_field(&arenas, h, "name").and_then(|f| value::read_str(&arenas, f)),
                value::record_field(&arenas, h, "value").and_then(|f| value::read_str(&arenas, f)),
            ) {
                builder = builder.header(name, val);
            }
        }
    }
    builder.body(Full::new(body)).unwrap_or_else(|_| {
        status(
            StatusCode::INTERNAL_SERVER_ERROR,
            b"cdz-http-gateway: invalid response headers\n",
        )
    })
}

/// Turn a program's terminal `http.response-cas` `Break` reason — a `ResponseCas.ResponseCas` value
/// (`status: Int64`, `headers: List(Header)`, `body_hash: Bytes`, §6) — into the HTTP response by FETCHING the
/// body from the control-shipped CAS: the handler answered with a blob hash (a `CasRef`), so the edge resolves
/// it here and serves the bytes. A malformed value or a `body_hash` that is not a 33-byte hash ⇒ `500`; a blob
/// the CAS does not hold, or a CAS fetch failure ⇒ `502` (the handler named a body the edge could not produce).
async fn decode_response_cas(cas: &Arc<dyn BlobStore>, reason: &[u8]) -> Response<Full<Bytes>> {
    let Some(arenas) = value::decode(reason) else {
        return status(
            StatusCode::INTERNAL_SERVER_ERROR,
            b"cdz-http-gateway: undecodable program response\n",
        );
    };
    let root = arenas.root;
    let Some(hash) = value::record_field(&arenas, root, "body_hash")
        .and_then(|f| value::read_bytes(&arenas, f))
        .and_then(|b| Hash::try_from(b.as_ref()).ok())
    else {
        return status(
            StatusCode::INTERNAL_SERVER_ERROR,
            b"cdz-http-gateway: response-cas has no valid body_hash\n",
        );
    };
    let body = match cas.get(hash).await {
        Ok(Some(bytes)) => bytes,
        Ok(None) => {
            return status(
                StatusCode::BAD_GATEWAY,
                b"cdz-http-gateway: response-cas body blob absent from CAS\n",
            );
        }
        Err(_) => {
            return status(
                StatusCode::BAD_GATEWAY,
                b"cdz-http-gateway: response-cas body fetch failed\n",
            );
        }
    };
    // status + headers decode mirrors `decode_response` (they differ only in where `body` comes from).
    let code = value::record_field(&arenas, root, "status")
        .and_then(|f| value::read_uint(&arenas, f))
        .and_then(|u| u16::try_from(u).ok())
        .and_then(|u| StatusCode::from_u16(u).ok())
        .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    let mut builder = Response::builder().status(code);
    if let Some(headers) =
        value::record_field(&arenas, root, "headers").and_then(|f| value::read_list(&arenas, f))
    {
        for &h in headers {
            if let (Some(name), Some(val)) = (
                value::record_field(&arenas, h, "name").and_then(|f| value::read_str(&arenas, f)),
                value::record_field(&arenas, h, "value").and_then(|f| value::read_str(&arenas, f)),
            ) {
                builder = builder.header(name, val);
            }
        }
    }
    builder.body(Full::new(body)).unwrap_or_else(|_| {
        status(
            StatusCode::INTERNAL_SERVER_ERROR,
            b"cdz-http-gateway: invalid response headers\n",
        )
    })
}

/// Turn a program's terminal `http.deny` `Break` reason — a `Deny.Deny` value (`status: Int64`, `reason:
/// Bytes`, §4) — into the HTTP response: floor with the HANDLER-SUPPLIED `status` and its plain-text `reason`
/// body ("status is the HTTP status to floor with, e.g. 403/404/429; reason a short plain-text body"). A
/// malformed value, a missing field, or a status outside the valid HTTP range falls back to a plain `403` — a
/// deny is still a deny even when its detail is unreadable.
fn decode_deny(reason: &[u8]) -> Response<Full<Bytes>> {
    let denied = || status(StatusCode::FORBIDDEN, b"cdz-http-gateway: denied\n");
    let Some(arenas) = value::decode(reason) else {
        return denied();
    };
    let root = arenas.root;
    let Some(body) =
        value::record_field(&arenas, root, "reason").and_then(|f| value::read_bytes(&arenas, f))
    else {
        return denied();
    };
    let code = value::record_field(&arenas, root, "status")
        .and_then(|f| value::read_uint(&arenas, f))
        .and_then(|u| u16::try_from(u).ok())
        .and_then(|u| StatusCode::from_u16(u).ok())
        .unwrap_or(StatusCode::FORBIDDEN);
    Response::builder()
        .status(code)
        .body(Full::new(body))
        .unwrap_or_else(|_| denied())
}

/// A static-body response with `code` and a plain-text `body` — the gateway's own floors (`503`/`400`/`403`/
/// `500`), distinct from a program's answer.
fn status(code: StatusCode, body: &'static [u8]) -> Response<Full<Bytes>> {
    Response::builder()
        .status(code)
        .body(Full::new(Bytes::from_static(body)))
        .expect("a static-body response is valid")
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
    fn an_unconfigured_gateway_answers_503_waiting_for_control() {
        // The "waiting for control" status when no config has been applied — a pure-leaf check of the floor
        // (the full serve/drive path is proven by the scripted conformance harness, design §7).
        assert_eq!(
            status(
                StatusCode::SERVICE_UNAVAILABLE,
                b"cdz-http-gateway: waiting for control (no program configured yet)\n"
            )
            .status(),
            StatusCode::SERVICE_UNAVAILABLE
        );
    }

    #[test]
    fn body_ceiling_flags_a_content_length_over_16_mib_and_floors_413() {
        // §8 #5: a DECLARED Content-Length over the ceiling is rejected (→ 413 before routing); at/under and
        // absent are not (the read-side Limited still bounds the actual bytes). The full oversized-body → 413
        // path is proven by the conformance harness (v-gateway-conformance's scenario).
        assert!(!content_length_exceeds_ceiling(None));
        assert!(!content_length_exceeds_ceiling(Some(0)));
        assert!(!content_length_exceeds_ceiling(Some(
            MAX_REQUEST_BODY as u64
        )));
        assert!(content_length_exceeds_ceiling(Some(
            MAX_REQUEST_BODY as u64 + 1
        )));
        assert!(content_length_exceeds_ceiling(Some(u64::MAX)));
        assert_eq!(
            status(StatusCode::PAYLOAD_TOO_LARGE, b"too large\n").status(),
            StatusCode::PAYLOAD_TOO_LARGE
        );
    }

    #[test]
    fn redial_backoff_doubles_from_250ms_and_caps_at_5s_without_overflow() {
        // The schedule the persistently-unreachable-control redial follows: a 250 ms floor doubling per
        // consecutive failure, capped at 5 s. `failures == 0` (a redial after a successful dial) is the floor.
        assert_eq!(redial_backoff(0), Duration::from_millis(250));
        assert_eq!(redial_backoff(1), Duration::from_millis(500));
        assert_eq!(redial_backoff(2), Duration::from_millis(1000));
        assert_eq!(redial_backoff(3), Duration::from_millis(2000));
        assert_eq!(redial_backoff(4), Duration::from_millis(4000));
        // 250 << 5 = 8000 ms clamps to the 5 s cap, and every larger count stays capped …
        assert_eq!(redial_backoff(5), Duration::from_millis(5000));
        assert_eq!(redial_backoff(100), Duration::from_millis(5000));
        // … including counts past the shift width, which must clamp (not overflow-panic).
        assert_eq!(redial_backoff(u32::MAX), Duration::from_millis(5000));
    }

    #[test]
    fn decode_deny_floors_with_the_denys_own_status_and_reason_not_a_fixed_403() {
        use cdz_platform::contracts::http_deny as denyc;
        // A well-formed Deny{404, "not found"} floors with 404 — the gateway READS the deny's status field
        // (design §4), it does not hard-code 403 (the router denies unmatched routes with deny(404, ...)).
        let mut b = value::ValueBuilder::new();
        let st = value::uint_leaf(&mut b, 404);
        let rs = value::bytes_leaf(&mut b, b"not found");
        let deny = denyc::deny_deny(
            &mut b,
            denyc::DenyDeny {
                status: st,
                reason: rs,
            },
        );
        let bytes = value::finish(b, deny, "Deny");
        assert_eq!(decode_deny(&bytes).status(), StatusCode::NOT_FOUND);

        // A 502 deny (the router's compile-error terminal) floors with 502, likewise honored.
        let mut b2 = value::ValueBuilder::new();
        let st2 = value::uint_leaf(&mut b2, 502);
        let rs2 = value::bytes_leaf(&mut b2, b"bad gateway");
        let deny2 = denyc::deny_deny(
            &mut b2,
            denyc::DenyDeny {
                status: st2,
                reason: rs2,
            },
        );
        let bytes2 = value::finish(b2, deny2, "Deny");
        assert_eq!(decode_deny(&bytes2).status(), StatusCode::BAD_GATEWAY);

        // An undecodable reason, and a status outside the valid HTTP range, both fall back to a plain 403 —
        // a deny is still a deny even when its detail is unreadable.
        assert_eq!(decode_deny(b"not a value").status(), StatusCode::FORBIDDEN);
        let mut b3 = value::ValueBuilder::new();
        let st3 = value::uint_leaf(&mut b3, 9999);
        let rs3 = value::bytes_leaf(&mut b3, b"weird");
        let deny3 = denyc::deny_deny(
            &mut b3,
            denyc::DenyDeny {
                status: st3,
                reason: rs3,
            },
        );
        let bytes3 = value::finish(b3, deny3, "Deny");
        assert_eq!(decode_deny(&bytes3).status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn decode_response_cas_fetches_the_body_from_the_cas_by_hash_else_502() {
        use async_trait::async_trait;
        use cdz_platform::contracts::http_response_cas as casc;
        use cdz_platform::{BlobStore, BlobStoreError, Hash};

        // A CAS that holds exactly one blob, under a known 33-byte hash.
        struct OneBlob {
            hash: Hash,
            bytes: Bytes,
        }
        #[async_trait]
        impl BlobStore for OneBlob {
            async fn put(&self, _b: Bytes) -> Result<Hash, BlobStoreError> {
                unreachable!("decode_response_cas never puts")
            }
            async fn get(&self, hash: Hash) -> Result<Option<Bytes>, BlobStoreError> {
                Ok((hash.as_bytes() == self.hash.as_bytes()).then(|| self.bytes.clone()))
            }
            async fn has(&self, _h: Hash) -> Result<bool, BlobStoreError> {
                Ok(true)
            }
        }

        let raw = [7u8; Hash::LEN];
        let cas: Arc<dyn BlobStore> = Arc::new(OneBlob {
            hash: Hash::from_bytes(raw),
            bytes: Bytes::from_static(b"blob-body-from-cas"),
        });

        // Build a ResponseCas{200, [], body_hash=raw}; the edge fetches the blob and answers 200.
        let build = |hash_bytes: &[u8]| -> Bytes {
            let mut b = value::ValueBuilder::new();
            let status = value::uint_leaf(&mut b, 200);
            let headers = value::list_value(&mut b, Vec::new());
            let body_hash = value::bytes_leaf(&mut b, hash_bytes);
            let rc = casc::response_cas_response_cas(
                &mut b,
                casc::ResponseCasResponseCas {
                    status,
                    headers,
                    body_hash,
                },
            );
            value::finish(b, rc, "ResponseCas")
        };
        assert_eq!(
            decode_response_cas(&cas, &build(&raw)).await.status(),
            StatusCode::OK
        );
        // A hash the CAS does not hold → 502 (handler named a body the edge can't produce).
        assert_eq!(
            decode_response_cas(&cas, &build(&[9u8; Hash::LEN]))
                .await
                .status(),
            StatusCode::BAD_GATEWAY
        );
        // An undecodable reason → 500.
        assert_eq!(
            decode_response_cas(&cas, b"not a value").await.status(),
            StatusCode::INTERNAL_SERVER_ERROR
        );
    }

    #[test]
    fn classify_frame_demuxes_config_and_down_by_contract_id_with_a_bare_fallback() {
        use cdz_http_protocol::ControlDown;
        let codec = control_frame_codec();
        let config = sample_config();

        // A bare (pre-flip, untagged) ControlConfig is still accepted AS a Config frame (rolling upgrade).
        let bare = encode_control_config(&config);
        assert!(matches!(
            classify_frame(&codec, &bare),
            Some(ControlFrame::Config(c)) if c == config
        ));

        // A tagged Config frame → demuxed to Config by its contract-id tag.
        let enveloped = codec.encode(&ControlFrame::Config(config.clone()));
        assert!(matches!(
            classify_frame(&codec, &enveloped),
            Some(ControlFrame::Config(c)) if c == config
        ));

        // A tagged Down frame → demuxed to Down (NOT mistaken for a config) — the branch that routes a
        // control message to a session's handler.
        let down = ControlDown {
            session: Bytes::from_static(b"sess-1"),
            correlation: Bytes::from_static(b"c1"),
            payload: Bytes::from_static(b"opaque"),
        };
        let down_frame = codec.encode(&ControlFrame::Down(down.clone()));
        assert!(matches!(
            classify_frame(&codec, &down_frame),
            Some(ControlFrame::Down(d)) if d == down
        ));

        // Garbage → no frame (dropped, not a panic).
        assert!(classify_frame(&codec, b"not a frame").is_none());
    }
}
