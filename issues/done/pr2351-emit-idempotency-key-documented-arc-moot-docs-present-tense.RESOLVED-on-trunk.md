# PR #2351 review — cdz-agent-host/src/emit.rs (v-agent-harness-host) — OPEN — 1 provenance (MED) + 1 efficiency (LOW) + 1 doc (LOW) [VERIFIED]

https://github.com/camshaft/cadenza/pull/2351 (EmitExecutor — cross-session messaging host-side routing, the
operator's cross-session-messaging next; branch cand/v-agent-harness-host-97a17edf70d9). Copilot 3 inline.

## c2 — `perform` drops the dispatch `idempotency_key` (`_idempotency_key`); for a side-effecting emit (sends to another session) the key matters for CRASH-RECOVERY + provenance (Copilot, emit.rs:55, also 98) — provenance [VERIFIED, MED]
> `perform` receives the dispatch `idempotency_key`, but it is currently marked unused
> (`_idempotency_key`). Since `emit` is side-effecting (it sends to another session), keeping the key
> available is important for crash-recovery semantics and provenance.
VERIFIED: `async fn perform(&mut self, req: &EffectRequest, _idempotency_key: Hash)` (diff:96) discards the
key (also the second executor at :98). For an at-least-once cross-session send, the idempotency key is what
lets a redelivered emit be de-duped on crash-recovery + traced for provenance — dropping it means a replay
after a crash can double-deliver to B's inbox. MED (the highest-value of the three; it's the
correctness-of-delivery-semantics question, not just style). RELAYED with the reachability caveat: whether
the current routing already de-dups elsewhere (or emit is exactly-once by construction) is v-ah-host's call —
but a side-effecting executor that ignores the idempotency key it's handed is worth a deliberate decision
(use it in the dedup/provenance path, or document why it's safe to drop). Coordinate w/ v-agent-harness (owns
the EffectKind::Emit + Inbound kernel side).

## c3 — `SessionId::new(req.target.as_ref())` re-allocates from a cheap-clone `Arc<str>` (Copilot, emit.rs:73) — efficiency [VERIFIED, LOW]
> `EffectRequest.target` is an `Arc<str>` (cheap-clone). Converting it to `&str` and then back into an
> `Arc<str>` via `SessionId::new(req.target.as_ref())` allocates a new string; cloning the existing
> `Arc<str>` avoids the extra allocation.
VERIFIED: `let target = SessionId::new(req.target.as_ref())` (diff:114) round-trips `Arc<str>`→`&str`→new
alloc. LOW efficiency. Fix: if `SessionId` wraps `Arc<str>`, construct it by cloning the existing Arc
(`SessionId::from(req.target.clone())` or similar) — no new allocation per emit.

## c1 — module docs carry staging language ("operator's next"/"v2") that goes stale vs the "document current behavior" guideline (Copilot, emit.rs:16) — doc [VERIFIED, LOW]
VERIFIED: module header has forward-looking/staging notes. LOW doc-hygiene. Fix: rewrite the header to
describe the CURRENT routing semantics (+ `cause` provenance once applied) without future-looking notes.

v-agent-harness-host owns cdz-agent-host. PR OPEN → all foldable pre-merge. c2 (idempotency_key) is the one
that matters — a delivery-semantics decision, not a nit.
