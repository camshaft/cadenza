//! The HTTP outpost gateway — a **dumb** gateway that boots from a control server and drives a
//! control-shipped, content-addressed Cadenza program as its router.
//!
//! Authoritative design: `implementation/design/DESIGN-http-outpost-drive-contract.md` (+ the original
//! `DESIGN-http-outpost.md` and the conformance harness doc). The gateway holds NOTHING of its own routing
//! logic:
//!
//!  - It is launched knowing only where the control server is. It dials a single **persistent,
//!    bidirectional** WebSocket to control (§3) and applies the `ControlConfig` control ships on connect
//!    (a CAS URL + credential + the root-router `ProgramHash`).
//!  - It resolves programs from the **content-addressed store** by hash ([`HttpBlobStore`], the shared
//!    `cdz-cas-http` client), and drives the root-router program as a **looping reducer** on the
//!    `system.rs` mailbox / fire-and-forget event-loop model (§1) — never a serial await-each-effect loop.
//!  - Routing is BAKED INTO the compiled root-router program and shipped by hash (§4); a route change
//!    recompiles the router and pushes a new hash (a live swap of the configured hash, no restart).
//!  - The program emits **effects** the gateway routes by contract-id (§2, payload opaque): `http.dispatch`
//!    (spawn + drive a subprogram by hash), `http.response` (answer; body `Inline | CasRef(hash)`, §6),
//!    `ws.send`, `http.deny`, `control.send` (opaque payload forwarded UP the control link), and timers
//!    (any request with a `deadline`).
//!
//! Wire frames (`ControlConfig` / `ControlUp` / `ControlDown` + the value-form codec) live in the shared
//! `cdz-http-protocol` crate and are consumed here — the gateway does not define its own copy.
//!
//! Behavior is proven by the scripted `v-gateway-conformance` integration harness (ML-surface run specs
//! driving CAS + `cdz-http-control-mock` + this gateway over real sockets), NOT rust `#[test]`s (§7).
//!
//! This crate is a clean rewrite (operator directive 2026-09-10: nuke the prior implementation and rebuild
//! from the design). Modules are added one landable slice at a time as the drive loop, effect resolver,
//! boot-from-control, live-swap, and CasRef body resolution land.

/// The HTTP content-addressed store client (design §3/§6): a [`cdz_platform::BlobStore`] backed by an HTTP
/// CAS at the control-supplied URL + credential, so the gateway fetches programs, deps, and `CasRef` bodies
/// by hash (base62 keys, digest-verified on 200). Re-exported from the shared `cdz-cas-http` crate
/// (`v-cas-http`) — one client, one wire. Construct
/// `HttpBlobStore::new(cas_url).with_read_credential(cas_credential)`.
pub use cdz_cas_http::HttpBlobStore;
