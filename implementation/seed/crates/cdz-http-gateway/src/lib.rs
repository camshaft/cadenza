//! The standalone HTTP outpost (`design/DESIGN-http-outpost.md`).
//!
//! An inbound HTTP/WebSocket edge served by content-addressed wasm handlers. The native edge parses each
//! request into an `http-request` value, a wasm reducer folds it, and the edge serializes the
//! `http-response` the fold produces back to the socket. Nothing about HTTP reaches below the edge — the
//! handlers are ordinary reducers, decoupled from the still-unsettled core platform (design §0.1).
//!
//! Built bottom-up, one increment per the design's §8 arc:
//!  - **P1a (here): [`codec`]** — the `http-request`/`http-response` value-form codec: Rust mirrors of the
//!    two userspace contracts (`cdz-platform/contracts/userspace/http-{request,response}.cdz`) and the
//!    encode/decode against `cadenza-ast`, in the exact canonical form the compiler's `Value.encode`/
//!    `Value.decode` produce (so a value the gateway emits is decodable by a Cadenza guest, and a guest's
//!    response decodes here).
//!  - **P1b (here): [`runner`]** — the per-request handler runner: instantiate a fresh session via a
//!    [`cdz_platform::ProgramStore`], deliver the request as `on_message`, read the `http-response` off
//!    the closing `Break`. Generic over the store, so it drives the wasmtime-backed store in production and
//!    a native test store in tests.
//!  - **P1c-1: [`gateway`]** — the router + request-serving core: match an [`HttpRequest`] against a route
//!    table to a handler, fold it through the runner, synthesize the `404`/`500` floors.
//!  - **P1c-2 (here): [`edge`]** — the native `hyper` HTTP/1 edge: bind a port, parse each request into an
//!    [`HttpRequest`], serve it through the [`gateway::Gateway`], serialize the response. Host plumbing only.
//!    (P1c-3 seeds the route table from a mock control server.)
//!  - **P1c-3 (in progress): [`wasm`]** (behind the `host` feature) — the wasmtime-backed handler store:
//!    reuse `cdz-platform`'s `WasmProgramStore` to instantiate a real content-addressed wasm handler per
//!    request (fresh in-memory `state`), no core platform. The mock control server + compiled-guest e2e
//!    build on this.
//!  - P1d: per-request resource bounds (epoch deadline + memory ceiling), no-cross-request-state-leak.

pub mod codec;
pub mod edge;
pub mod gateway;
pub mod runner;

/// The per-connection WebSocket session driver (design §6): folds `ws-event`s through a session reducer
/// and collects the `ws-send` frames it pushes. Socket-independent (generic over [`cdz_platform::ProgramStore`]);
/// the hyper WebSocket upgrade + framing that feeds it is a later slice.
pub mod ws;

/// The control-link client (design §2/§3, P3): dials the control server over a WebSocket and receives its
/// route table as a `cadenza-ast` frame. Wasm-free transport (tokio + tokio-tungstenite + the frame codec),
/// so it sits outside the `host` feature — the real ws-dialed counterpart of the in-process mock control
/// server ([`control`]).
pub mod control_link;

/// The wasmtime-backed handler store — behind the `host` feature (off by default so the core spine build
/// stays wasmtime-free). Reuses `cdz-platform`'s `WasmProgramStore` (design §0.1).
#[cfg(feature = "host")]
pub mod wasm;

/// The HTTP content-addressed store client (design §3, dumb-gateway redirect) — behind the `host` feature.
/// A [`cdz_platform::BlobStore`] backed by an HTTP CAS (control-server-supplied URL + credential): the
/// gateway fetches programs + deps by hash, content-verified, in place of a local/in-memory store.
#[cfg(feature = "host")]
pub mod cas_http;

/// The mock control server (design §3) — behind the `host` feature (it assembles a wasmtime-backed edge).
/// Ships a route table + handler blobs and builds a ready-to-serve [`edge::HttpEdge`] from them, standing
/// in for the real ws-dialed control server for end-to-end tests. Also home to [`control::assemble_edge`],
/// the shared boot step (frame + components → edge).
#[cfg(feature = "host")]
pub mod control;

/// Deployment boot — behind the `host` feature. Stands up the gateway as a runnable server from a local
/// deployment directory (a `route-table.bin` frame + `*.wasm` components); the `cdz-http-gateway` binary.
#[cfg(feature = "host")]
pub mod boot;
