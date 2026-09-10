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
//!  - P1d: per-request resource bounds (epoch deadline + memory ceiling), no-cross-request-state-leak.

pub mod codec;
pub mod edge;
pub mod gateway;
pub mod runner;
