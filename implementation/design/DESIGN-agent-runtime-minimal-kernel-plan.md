# Minimal-kernel migration plan — Rust→Cadenza, decomposed into gated sub-rungs (K1–K5)

**Owner:** v-agent-harness. **Builds on:** `DESIGN-agent-runtime-minimal-kernel.md` (the re-charter + audit + the
5 forks). **Status:** PLAN. Each rung is one gated MR. Rungs are tagged **[fork-independent]** (buildable now,
no operator ruling needed) or **[blocked: fork N]** (needs the operator's ruling on fork N first). I proceed on
the fork-independent rungs and do NOT wait for the reply (mandate: keep working).

## Fork #2 (the load-bearing one) — a recommendation, not just an open question

The design doc flagged: *how does the Cadenza `interpret(tail, event) -> [HostOp]` program return a LIST of ops to
the kernel, given the host-op ABI can't return a compound?* The codeact-spike already settled the ABI reality
(`cadenza-agent-harness-codeact-spike`): **a host op returning String/List/compound is NOT expressible today**
(`backend/wasm/host.rs::abi_val_type` maps scalar results + string *params* only). So the plan does NOT block on
an ABI widening. Two ABI-independent mechanisms work with today's scalar-only host ops:

- **(A) Drain-loop / one-op-at-a-time.** `interpret` returns ONE op's index or a scalar "op-count", and the kernel
  reads each op from a scratch region the Cadenza program `append`ed. Concretely: the interpret program `append`s
  its intended HostOps AS EVENTS (kind `hostop`) to the log, then returns a scalar count; the kernel drains those
  `hostop` events and executes them. **This needs NO ABI change and is itself log-native** (the ops are events —
  auditable, replayable). The kernel still understands no event *kinds* — it's told "drain N hostop events" and
  each hostop event's payload is an opaque `{op, args}` the kernel's 4 primitives consume by position.
- **(B) Widen the host ABI** so a Cadenza fn returns a List of op-records (ties v-effects/v-rust-backend). Cleaner
  long-term, but a compiler change → NOT the floor, and it delays the whole re-charter behind another vertical.

**Recommendation to the operator: adopt (A) for v1** — it's ABI-independent, log-native, and keeps the kernel at
4 primitives. Revisit (B) later as an ergonomics optimization. This unblocks K1 without waiting on fork #2's
ruling (I'll build K1 against mechanism A; if the operator prefers B, K1's op-transport swaps but the rung shape
holds).

## The rungs

- **K1 — the tiny event-agnostic kernel skeleton. [fork-independent]**
  A `Kernel` that: `read_tail` → find the latest `program` event (the ONE hardcoded kind — the genesis, fork #5)
  → call it via `cdz_run` passing the opaque tail → drain the `hostop` events it appended (mechanism A) →
  execute each via the 4 primitives (`append`, `read_tail`, `invoke`, `schedule`). NO msg/sub/model knowledge.
  Reuses the already-generic `Log` trait + `cdz_run::run_agent_hosted` (both event-agnostic). Gated: a test with
  a trivial Cadenza `interpret` that emits one `append` hostop → kernel executes it → the event lands. This is
  the whole kernel; everything else is Cadenza. (Genesis's single hardcoded `program`-kind is the one Rust
  string that names an event — fork #5; K1 uses it, flagged for the operator.)

- **K2 — port msg.rs → Cadenza, gated against the Rust oracle. [blocked: fork #4 (keep-oracle vs reject-now)]**
  A Cadenza module `msg.cdz`: `Message`/`Ack` encode/decode, `inbox_for`, `reply_then_ack` — same wire format as
  the Rust `msg.rs` (so they're differential-equal). Gate: a differential test feeds the SAME log to the Rust
  `inbox_for` (the oracle) and the Cadenza `inbox_for`; assert identical. Keep the Rust until the Cadenza passes.
  (If fork #4 = reject-now, skip the oracle and gate the Cadenza against hand-written expectations instead.)

- **K3 — port sub.rs → Cadenza. [blocked: fork #4]**
  `sub.cdz`: `Subscription`/`Predicate`, `active_subscriptions`, `dispatch`, `matches`. Same differential-oracle
  discipline against the Rust `sub.rs`. Predicate matching becomes a pure Cadenza fn over the opaque event.

- **K4 — slim fold.rs to the generic driver. [fork-independent]**
  Move the `model-request`/`model-response` record/replay OUT of Rust: the Cadenza program emits the `append`s
  around an `invoke("cadenza:model/api", prompt)`. What remains in Rust is the generic interpret-loop + the
  `invoke` capability plumbing (reuses `RunOpts::host_responses` for replay — already generic). Gate: the L1c
  replay-determinism proof still holds, now with the model-event knowledge in Cadenza not Rust.

- **K5 — delete the Rust event code. [blocked: fork #4 + K2+K3+K4 landed]**
  Once the Cadenza msg/sub/model logic passes the differential oracle, DELETE `msg.rs` + `sub.rs` + the
  event-aware parts of `fold.rs`. The kernel is now just K1's skeleton + the log backends. Verify the whole
  agent-loop + inbox + subscription behavior runs entirely from the Cadenza program with the Rust reference gone.

## Ordering + what I build now

K1 and K4 are fork-independent → **build K1 first** (the skeleton makes the whole shape concrete + de-risks
mechanism A empirically). K2/K3/K5 wait on fork #4's keep-vs-reject ruling, but I can DRAFT the Cadenza `msg.cdz`
in parallel (it's needed either way; only the gating strategy changes). K4 can follow K1.

**This tick's follow-on:** start **K1** — the tiny kernel skeleton + the mechanism-A drain-loop + a trivial-
interpret gated test — since it's fork-independent and proves the core "host understands no events" claim.
