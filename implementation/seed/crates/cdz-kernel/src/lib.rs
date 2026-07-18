//! `cdz-kernel` — the log-native agent-runtime microkernel (agent-runtime L1).
//!
//! The vision (`implementation/design/DESIGN-agent-runtime-vision.md`) is a log-native agent OS: a minimal
//! core that TAILS an ordered log, FOLDS it with a Cadenza program, and EXECUTES the effect-requests the
//! fold emits — appending every non-deterministic result (a model call, a clock, a build) back as an
//! immutable event, so the fold over `(request-event, response-event)` is pure and replayable (§2.3).
//!
//! This module (**L1a**) is the foundation: the [`Log`] abstraction and a deterministic file-backed
//! implementation. Later rungs build the fold owner on top (L1b), the replay-determinism gate (L1c), and a
//! DynamoDB backend behind the `aws` feature (L1d). The [`Log`] trait is the seam those rungs — and the
//! future DynamoDB write plane (§2.1: a many-writer ordering authority) — implement, so the fold owner is
//! written once against the trait and the backend swaps underneath.

use anyhow::Result;

/// A monotonic, gap-free sequence number assigned by the log on append — the total order the fold reads in.
/// The DynamoDB write plane (L1d) assigns this via a conditional write (the ordering + dedup authority,
/// vision §2.1); the file log assigns it by append position. `seq` starts at 0 for the first event.
pub type Seq = u64;

/// One immutable log event: its `seq` (assigned by the log), a `kind` tag (what the event IS — e.g. a model
/// request, a model response, a message), and an opaque `payload` (the event body; a later rung fixes the
/// concrete encoding — L1a treats it as bytes so the log is agnostic to the fold program's event schema).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Event {
    pub seq: Seq,
    pub kind: String,
    pub payload: Vec<u8>,
}

/// The append-only log: the single source of truth the whole runtime folds over. `append` adds an event and
/// returns its assigned `seq`; `tail` returns every event with `seq >= from` in order. Deliberately minimal
/// — a fold owner needs exactly "add an event" + "read the ordered tail from a cursor". The write plane is
/// decoupled from the fold plane (vision §2.2), so many writers may `append` concurrently while one owner
/// `tail`s; a backend enforces the ordering authority (the file impl is single-process, DynamoDB is the
/// multi-writer authority at L1d).
pub trait Log {
    /// Append an event with `kind` + `payload`, returning its assigned monotonic `seq`.
    fn append(&mut self, kind: &str, payload: &[u8]) -> Result<Seq>;

    /// Return every event with `seq >= from`, in ascending `seq` order. `tail(0)` returns the whole log.
    fn tail(&self, from: Seq) -> Result<Vec<Event>>;
}

/// A deterministic FILE-backed [`Log`] (the L1a backend — local, no network, CI-safe). Events are appended
/// as length-prefixed records to a single file, so the on-disk order IS the seq order and a fresh process
/// re-reads the same sequence. This is the stand-in for the DynamoDB log (L1d) while the fold owner + the
/// replay-determinism gate (L1b/L1c) are built against the [`Log`] trait — the same mock-first discipline
/// the Bedrock embedder shipped with.
pub mod file_log;
pub use file_log::FileLog;

/// The single-threaded FOLD OWNER (L1b): tails the log, drives one agent-loop turn through the embedder,
/// and appends every model effect-request/response as an immutable event (recorded-effect determinism,
/// vision §2.3) — the "tail → fold → execute effect-request → append" core.
pub mod fold;

/// The DynamoDB log backend (L1d): the real many-writer ordering authority (vision §2.1). The event↔item
/// MARSHALLING is pure + tested in the default build; the DynamoDB client is behind the `aws` feature.
pub mod dynamo_log;

/// Messaging over the log (L2a): a [`msg::Message`]/[`msg::Ack`] is a typed durable event (vision §9); the
/// inbox becomes a fold over these (L2b). Pure encode/decode to the [`Event`] payload — no queue.
pub mod msg;

/// Subscriptions over the log (L3): a [`sub::Subscription`] `{id, predicate, program_ref, capability}` is a
/// durable `subscribe` event (vision §8, the one reactive primitive). L3a is the type + a concrete matchable
/// [`sub::Predicate`] + a pure codec; L3b ([`sub::active_subscriptions`]) folds the active set (supersession
/// by id + `unsubscribe` revocation); L3c ([`sub::dispatch`]) selects the active subscriptions a landed event
/// matches — the schedulable set (running the handlers under their capability is the fold-owner rung, L4/L5).
///
/// NOTE (re-charter): `msg` + `sub` above are the pre-re-charter EVENT-SPECIFIC Rust (L2/L3), retained as the
/// differential ORACLE the Cadenza ports gate against, to be deleted once those pass. New event logic is
/// Cadenza (see [`kernel`]).
pub mod sub;

/// External adapters over the log (rung KA — the connector topology): a separate deployable (e.g. a Slack
/// connector) with TWO logs — INBOUND ([`connector::post_on_behalf`]) posts a user's message into the MAIN log
/// on-behalf-of a Cedar principal; OUTBOUND ([`connector::work_items`]) reads the connector's OWN kernel-
/// written log (separate-stream, NOT tailing the main log — operator connector-logs ruling). Pure over `Log`;
/// the real Slack HTTP I/O is the deployed binary on top.
pub mod connector;

/// The tiny KERNEL (minimal-kernel re-charter, rung K1): compile a self-modifiable Cadenza `interpret`
/// program AT RUNTIME (embedded rcdzc) into a provider, compose it with a peer executor, and run it (cdz-run's
/// wasmtime) so interpret's `(List HostOp)` result crosses to the executor as a handle over the shared runtime
/// — NO host-ABI widening (the peer path v-effects + v-peer-linking verified). The daemon understands NO
/// events; all event MEANING is in the Cadenza interpret program. This is the codeact-spike shape (compiler +
/// wasmtime in-process) realized as the deploy-once kernel. See `DESIGN-agent-runtime-minimal-kernel.md`.
pub mod kernel;

/// Bootstrap + genesis injection (rung KC — the thin CLI's log half): [`boot::inject_genesis`] seeds the log
/// with the genesis interpret SOURCE as a `program` event; [`boot::latest_program`] is the kernel's genesis
/// lookup (the latest `program` event's source). No hardcoded genesis — the first program is data in the log
/// the CLI injects (operator fork-5 ruling), and a later `program` event self-supersedes it.
pub mod boot;

/// Capability policies in the log (capability-attenuation — operator ruling): [`policy::append_policy`] writes
/// a CEDAR policy doc to the log as a `policy` event; [`policy::latest_policy`] is the invocation path's
/// lookup (the latest `policy` event's Cedar text). The kernel grants broad primitives; the Cedar doc from the
/// log attenuates each invocation to its minimal privilege set. The log-store half; evaluation is `cdz_agent::cedar`.
pub mod policy;

/// The daemon loop (rung KC-daemon — the "start the daemon" half of fork-5): [`daemon::tick`] runs one step —
/// read the log, find the latest genesis `program` ([`boot::latest_program`]), and drive an interpret turn
/// ([`kernel::run_interpret`]). Ties the boot half (genesis lookup) + the kernel half (compile+run) into the
/// per-event loop spine; the event-source driver is thin on top. Pure over the `Log` + the runtime store.
pub mod daemon;

/// Run the compiler AS A WASM (K1-dep wasm-swap, operator #54): [`wasm_compiler::compile_via_wasm`] loads
/// `rcdzc.wasm` (the wasm32-wasip1 build of rcdzc) into wasmtime + a WASI ctx and drives its alloc→compile→
/// read→dealloc ABI to turn AST bytes into a program component — so the compiler is a swappable wasm artifact,
/// not native-linked. Behind the `wasm-compiler` feature (the default build uses native rcdzc). Plan (b) folds
/// this into cdz-run's Component API once rcdzc-wasm is re-authored as a component.
#[cfg(feature = "wasm-compiler")]
pub mod wasm_compiler;
