# The platform conformance suite — portable Cadenza sessions driven to a fixpoint by ONE kick-off event

Owner: TBD (a `vertical`, area `cdz-kernel`/`cdz-agent-host` + `xtask`/`cdz-corpus`). Design by
`design-platform-conformance`, revised + extended by `design-platform-conformance-msg`.
Status: **PROPOSAL — designed AUTONOMOUSLY, awaiting a build owner.** Operator sparks via concierge:

- **seq356** (verbatim): *"I'm wondering if we should spin up a vertical for the platform conformance
  suite. Like we should be able to define reducers in cadenza and effect handlers and make assertions
  around events being processed and end state and everything. … really getting a good portable test
  suite is worth its weight in gold. The compiler corpus suite has been incredibly valuable."*
- **seq357**: *"The design of the platform test suite needs to be autonomous."*
- **seq358**: *"We need to be able to have more than one session interacting as well so we can test out
  messaging."*
- **seq359** (the model refinement, verbatim): *"Instead of canned responses I would much prefer to just
  have a single kick off event that starts the whole thing and drives it to completion. We do not need
  to model anything custom since we have reducers and effect handlers as sessions. So they just need to
  respond and react and update their state."*

These are honored together: the suite is shaped without a live operator session (every fork carries a
chosen default; forks the operator may want to own are flagged **⟨operator may ratify⟩**), and the suite
runs UNATTENDED in CI — self-contained, deterministic, machine-checkable pass/fail, exactly the way the
compiler corpus runs under `cargo xtask gate`.

> **What seq359 changed (folded in below, old model deleted — no migration layer).** The first draft of
> this design (landed `bf8674a36`) drove a case with a SCRIPTED event tape (many `(deliver …)`) and
> SCRIPTED effect-handler responses (`(family "log" (respond (ok)))` canned tables). The operator does
> NOT want that. The revised model: a case provides **exactly one kick-off event**, and the constellation
> of sessions — each a real Cadenza reducer, including the effect handlers, which are themselves sessions
> — **runs organically to a fixpoint**, reacting and updating their own state, with NO canned responses
> and NO test-only modeling. This design has been rewritten to that model; the scripted-tape and
> scripted-response vocabulary is removed, not deprecated-in-place.

Subsystem: primarily `xtask` (the runner/grader) + `cdz-corpus` (the case reader) + a new spec tree
`spec/platform/`. It exercises `cdz-kernel` + `cdz-agent-host` (the reducer/event/KV/effect/messaging
machinery) as the system-under-test but adds NO kernel/host production code — they are dependencies, not
change targets. Coordinated with `corpus-bugfix` (owns the `cdz-corpus` reader + record model + gate
baseline mechanics — this suite REUSES that infrastructure; ownership split confirmed: corpus-bugfix
implements all reader edits to a co-designed grammar, the vertical owns the `xtask` grade path + the
`spec/platform` baseline), `v-agent-harness` (owns the kernel seam: the `Reducer`/`Executor` traits, the
WIT `cadenza:agent-kernel` world, the `Session` drive loop), `v-agent-harness-host` (owns `cdz-agent-host`:
`UserspaceEffectExecutor`, `CompositeExecutor` family routing, the `EmitExecutor` peer-messaging path, the
reply/settle round-trip), and `v-effects` (language-level effect vocabulary).

> **The one-sentence thesis.** The compiler corpus asks *"does this Cadenza PROGRAM compile+run to this
> value/error/trap?"*; the platform conformance suite asks *"given these Cadenza sessions (reducers +
> effect-handler sessions) and ONE kick-off event, do the sessions — reacting organically to a fixpoint —
> emit these effects and these inter-session messages in this order, and each settle to this end-state?"*
> The same recorded-oracle, backend-agnostic, `cargo xtask gate`-run discipline the compiler corpus uses,
> applied to the runtime/platform layer — but the driver is a single event feeding REAL reducer/handler
> logic run to quiescence, not a scripted event tape with canned responses.

## Why this is worth building (and why it's cheap)

Three findings make this a high-leverage, low-risk vertical:

1. **A platform conformance run is already a small, proven shape.** `cdz-kernel`'s
   `kernel_e2e_tests/loop_and_recovery.rs` is a working reference: build a `Session::genesis(reducer,
   nonce)`, register effect handlers on a `CompositeExecutor`, feed an event via
   `deliver(EventBody::Inbound{..})`, then assert on events-processed (`session.event_count()`) and
   end-state (`session.kv().get(..)` / `status_snapshot()`). The suite turns that hand-written-Rust
   pattern into a **portable, declarative, Cadenza-defined case** the gate runs unattended.

2. **The effect-handler-as-session round-trip already exists.** `cdz-agent-host`'s
   `UserspaceEffectExecutor` + `effect_reply`/`reply_exec` + `effect_registry` already implement "an
   effect is served by another Cadenza session": a caller's effect is DEFERRED, delivered to the handler
   session as an `effect-request/<family>` inbound, the handler folds it and emits `effect/reply`, and the
   `ReplyExecutor` settles the reply onto the caller's open `EffectId`. seq359's "effect handlers as
   sessions" is EXACTLY this machinery — the suite consumes it, it does not build it.

3. **Cross-session messaging already exists.** `cdz-agent-host`'s `EmitExecutor` routes a session's `Emit`
   effect (family `"message"`, `target` = peer `SessionId`) to the peer's inbox as an
   `Inbound{family="message"}`, fire-and-forget, with a `delivery-failure` bounce to the sender when the
   target is gone. seq358's messaging is this seam; the suite makes it portably testable.

So the build is mostly (a) a single-kick-off case format, (b) a runner that assembles the session
constellation from a case and drives it to a fixpoint under a DETERMINISTIC scheduler, and (c) a grader —
riding infrastructure that already exists on the corpus side, the kernel side, and the host side.

## The design decisions (each carries a default)

### D1 — Format: EXTEND the corpus infrastructure with a new "session" case genre. ⟨operator may ratify⟩
**Chosen default: extend `cdz-corpus` + `cargo xtask gate` rather than stand up a parallel runner.**

The operator's own framing — *"worth its weight in gold … the compiler corpus has been incredibly
valuable"* — is a mandate to reuse what works. We inherit, for free: the case reader + record model, the
literate markdown surface (`cdz-corpus/src/markdown.rs`, so a case has a `.md` twin), the `.gate-baseline`
diff-not-count regression mechanics (what actually catches per-MR regressions), the `--save`/`--check`
flow, the differential-target infra, and the operator's existing mental model + tooling.

The suite lives in a NEW spec tree — **`spec/platform/NN-feature.sexp`** (mirrors `spec/semantics/`) —
with its OWN baseline files (`spec/platform/.gate-baseline*`). It is a new *genre* of case within the
same reader, not a new reader. A platform case is distinguished by a top-level `(platform-case …)` marker
(so the grade path dispatches on genre — corpus-bugfix flagged that a new genre wants its own record
marker) carrying `(session …)` blocks + a `(kickoff …)`, instead of a compiler `(case (input …))`.

> ⟨operator may ratify⟩ A fully independent `spec/platform/` runner stays open if the operator wants the
> platform vocabulary to evolve without touching the compiler corpus reader. Default is REUSE; the grammar
> seam is small enough to relocate later.

### D2 — The unit under test is a **Cadenza-defined reducer compiled to a wasm component**, driven through the real kernel; effect handlers are ALSO reducer sessions (seq359). ⟨operator may ratify⟩
**Chosen default: every session — agent OR effect-handler — is `Cadenza reducer → rcdzc → wasm component
→ cdz-kernel`, matching the corpus's default wasm target. There are NO canned response tables.**

This is the only choice that honors *"define reducers in cadenza"*, *"effect handlers as sessions"*, AND
*"portable"*: each reducer is authored in Cadenza, the same source a production agent would run, and
executes on the real kernel drive loop — not a hand-written Rust stand-in and not a canned-outcome table.
It rides toward the `DESIGN-binary-ast-abi.md` `apply(list<u8>) -> list<u8>` boundary that makes a reducer
definition portable across a Rust guest and a Cadenza guest.

seq359 collapses the old "two handler realizations" fork: the **scripted-response table is deleted**. An
effect is served by a handler SESSION — a Cadenza reducer bound to serve one or more effect families —
via the existing `UserspaceEffectExecutor` deferral + `effect/reply` settle. A case that wants a trivial
handler (e.g. "every `log` succeeds") writes a two-line Cadenza reducer that folds the effect-request and
replies `ok`; that is still a real, compiled, deterministic session, not a mock.

Portability ladder (mirrors the corpus's wasm → ML targets):
- **Now:** wasm reducers on the real kernel/host — the oracle.
- **Later (I5):** a second implementation (a Cadenza-ML-hosted kernel, or the binary-AST guest as a
  distinct backend) graded DIFFERENTIALLY against the oracle — no second baseline, exactly how
  `run_program_ml` grades the ML compiler against the wasm oracle today.

> ⟨operator may ratify⟩ Whether to keep a tiny inline-Rust-reducer fast-path for harness smoke tests.
> Default: NO — one path (Cadenza reducer), so the suite can never pass on a reducer that doesn't actually
> compile from Cadenza. The existing Rust `kernel_e2e_tests` remain as unit tests; this does not overlap.

### D3 — Case shape: N `(session <alias> …)` blocks + exactly ONE `(kickoff …)`; assertions are terminal (whole-run).
A case defines the constellation of sessions, binds which serve effect families as handlers, gives ONE
kick-off event, and asserts on the whole-run emitted-effect sequence, the inter-session message sequence,
and each session's end-state. Illustrative (grammar, not final bytes):

```
(platform-case "a worker asks a clock handler for the time, logs it, and messages a reporter"
  (doc "kickoff -> worker; worker performs `now` (served by the clock session) then `log`,
        then Emits a `done` message to the reporter; reporter folds it and records seen=1.")

  ;; The constellation. Each session is a real Cadenza reducer compiled to a component.
  (session "worker"   (reducer <worker-reducer>))
  (session "reporter" (reducer <reporter-reducer>))
  (session "clock"    (reducer <clock-handler-reducer>)   (serves "now"))   ;; a handler SESSION
  (session "logger"   (reducer <logger-handler-reducer>)  (serves "log"))   ;; a handler SESSION

  ;; The ONE kick-off event. Everything else happens organically.
  (kickoff "worker" (inbound "start" (: unit Unit)))

  ;; Whole-run assertions (deterministic order — see D5). None are scripted responses; they OBSERVE
  ;; what the real sessions did as the constellation ran to a fixpoint.
  (expect-effects                                    ;; order-verified effects dispatched across the run
    (effect (from "worker") (family "now"))
    (effect (from "worker") (family "log") (: "t=0" String)))
  (expect-messages                                   ;; order-verified inter-session messages delivered
    (message (from "worker") (to "reporter") (family "message") (: "done" String)))

  ;; Per-session terminal assertions, keyed by alias.
  (end-state "worker"   (status quiescent))
  (end-state "reporter" (kv "seen" (: 1 Int64)) (status quiescent)))
```

Clause vocabulary (the platform genre; each maps to an existing kernel/host accessor so grading is a
direct comparison, never a heuristic):

| Clause | Meaning | Anchor it grades against |
|---|---|---|
| `(session <alias> (reducer <prog>) [(serves <family>…)])` | one SUT session; `serves` binds it as the handler for those effect families | `HostedSession::genesis_with_nonce` + `effect_registry` / `UserspaceEffectExecutor` |
| `(kickoff <alias> (inbound <family> <value>))` | THE single kick-off event; the only external stimulus | `AgentHost::deliver(id_of(alias), Inbound{..}, None)` — called exactly once |
| `(expect-effects (effect (from <a>) (family <f>) [<value>])…)` | order-verified sequence of effects DISPATCHED over the whole run | `Dispatched` frames on each session's log (cf. corpus `(host-calls …)`) |
| `(expect-messages (message (from <a>) (to <b>) (family <f>) <value>)…)` | order-verified sequence of inter-session messages DELIVERED over the run | routed `Inbound{family="message"}` deliveries the runner observed |
| `(expect-delivery-failure (from <a>) (to <alias>) …)` | a bounce: Emit to a gone/unknown peer → sender folds `delivery-failure` | `bounce_delivery_failure` / `DELIVERY_FAILURE_FAMILY` |
| `(end-state <alias> (kv <key> <value>)…)` | per-session terminal KV assertions | `session(alias).kv().get(key)` |
| `(end-state <alias> (status <state>))` | per-session terminal status | `StatusSnapshot.state` (`Active/Quiescent/Stalled/Closed`) |
| `(end-state <alias> (closed (success <v>)))` | close outcome, if the session closed — I4 | `CloseOutcome` (`Success/Failure`) |
| `(events-processed <alias> <n>)` | per-session total appended log length | `Session::event_count()` |

Values everywhere use the corpus's canonical `(: value Type)` form, so the grader reuses the corpus
value-comparison, and reducer programs use the same homoiconic Cadenza s-expression the corpus `(input …)`
uses — so a platform case is authored, read, and diffed by the same machinery. A single-session case is
the degenerate `N=1` shape (one `(session …)`, no `serves`/`expect-messages`).

### D4 — Assertion surface: emitted-effect sequence + inter-session-message sequence + per-session end-state (KV + status); then close/recovery.
The FLOOR that makes the suite meaningful is **emitted-effect sequence + end-state KV** (the two the
operator named in seq356 — *"events being processed and end state"*) plus the **inter-session message
sequence** (seq358). `status` is cheap (same snapshot). `closed`/`recovers` are I4 (they need lifecycle +
the durable-log recovery gate, themselves in-flight designs). Every assertion OBSERVES the real run; none
scripts a response.

### D5 — Determinism WITHOUT scripting (the seq359 puzzle, solved).
seq359 removes the scripted tape/responses but keeps the CI-reproducibility requirement. The organic run
is still fully deterministic because the constellation is a **closed, pure, deterministically-scheduled
rewrite system** seeded by one event. The four invariants the runner enforces:

1. **Reducers are pure folds.** A reducer is a pure function of `(event, kv)` (kernel §17 totality). Given
   a fixed event order, every session's trajectory is determined. Nothing in a fold reads outside its
   inputs.
2. **A deterministic scheduler fixes the event order (MD-SCHED).** The runner does NOT use the production
   `AsyncAgentHost::run` loop — that multiplexes with tokio `select!`, which by its own doc gives **NO
   ordering/fairness guarantee**. Instead the runner drives the lower primitive
   `AgentHost::deliver(session, body, cause)` itself and processes a single in-memory queue in **strict
   FIFO**: deliver the one `(kickoff …)` inbound → drive that session to quiescence → append, in emission
   order, every effect it dispatched (routed to the `serves` handler session as an `effect-request/<f>`
   inbound) and every `message` it Emitted (routed to the target as a `message` inbound) → drain the queue
   FIFO, each delivery driven to quiescence (which may enqueue more effect-requests / replies / messages),
   and each handler `effect/reply` settled onto the caller's open `EffectId` as an `EffectResult` inbound.
   This is a deterministic breadth-first drive to a global **fixpoint**: DONE when the queue is empty and
   every session is quiescent. A fixed, replay-stable interleaving — no `select!`, no wall clock.
3. **No clock, no randomness, no network.** There is no live executor: every effect family a reducer can
   emit is served by a handler SESSION declared in the case (`serves`). An effect for an UNSERVED family
   is a **case failure** (surfaced, mirroring `CompositeExecutor`'s observable-Err-on-unroutable, §9d), so
   a reducer can never reach a real clock/network. Deterministic session ids: `SessionId` IS the genesis
   `Hash`, and the runner supplies each session's spawn nonce as `Hash::of(case_salt ++ alias)` (via
   `genesis_with_nonce` / `derive_genesis_hash`) — NOT OS entropy — so ids are a pure function of the case,
   identical every run and in CI. Timers, if a case needs them, are out of the FLOOR (I2+): the FLOOR
   forbids timer/clock effects, so the FLOOR run has no time axis at all.
4. **Bounded, so a livelock is a Fail not a hang.** Each fold runs under the kernel fuel budget
   (exhaustion → a recorded `FoldFailed`, gradeable). The global drive runs under a per-case **step
   budget** (total deliveries); exceeding it (an unbounded effect/message ping-pong) is a recorded
   `SettleUnbounded` Fail, never a stuck CI job.

The verdict is a per-case `Pass/Todo/Fail` rolled into `spec/platform/.gate-baseline`; a `pass →
not-pass` flip or a vanished case fails `gate --check`; newly-passing cases are reported, not fatal — so
the vertical grows the suite unattended without renegotiating a pass-count each MR.

### MD1 — Peer/handler addressing: sessions are named by a stable local ALIAS; the runner binds alias → deterministic SessionId.
A case addresses peers and handlers WITHOUT hardcoding a genesis-hash-hex (content-derived, would churn on
any reducer edit). Each session has a short **alias** (`"worker"`, `"clock"`); the runner assigns the
deterministic id (D5.3) and threads the alias→id map to the reducers so a sender can name its target.

- **How a reducer learns a peer's id.** Before the single kick-off, the runner delivers each session its
  `(alias, SessionId-hex)` bindings as a genesis-CONFIGURATION `Inbound` (family `platform/peers`, folded
  into KV under a well-known key) — mirroring the host's existing genesis-setup seed path
  (`genesis_ct::CONTEXT`, delivered as ordinary early inbound events). This is session CONFIGURATION, not
  part of the interaction: it is distinct from the one `(kickoff …)` stimulus, so "single kick-off event"
  is honored (config sets the constellation up; the kick-off starts the interaction). A reducer emitting
  to `"reporter"` reads `kv["peers"]["reporter"]` for the target id; the runner rewrites the case's
  alias-addressed `expect-*` clauses to/from ids at the grading boundary, so the author never sees a hash.

> ⟨operator may ratify⟩ Alternative: a name-directory effect (send to `"reporter"` directly, host
> resolves). Default is alias→id-in-KV because it uses the EXISTING genesis-context seed + the Emit
> target-is-SessionId contract with no new kernel/host surface. If the OUTPOST federation later adds a
> `session-directory` lookup effect, the suite adopts it then.

### MD2 — Relation to the OUTPOST / federation (`v-agent-harness-host`).
The OUTPOST (`design-the-outpost-host-websocket-federation-node`) is the PRODUCTION cross-node router: it
carries the same `Emit`→peer-`Inbound` messaging and the same effect-handler-session deferral over a
WebSocket wire between federation nodes. This suite is the **in-process, deterministic conformance oracle**
for that behavior: the semantics (family `"message"`, target=SessionId, fire-and-forget, delivery-failure
bounce, cause provenance; effect deferral → `effect-request` → `effect/reply` settle) are identical whether
routed in-process or over ws-transport, so a constellation that passes a `spec/platform/` case behaves
identically when federated. The vertical coordinates the messaging + handler-session increments with
`v-agent-harness-host` so the OUTPOST's wire-level tests and this suite's declarative cases assert the SAME
contract from the two ends (wire vs. semantics). The suite adds no host/kernel production code.

## Increments (top-to-bottom, the way a vertical lands them)

Each increment is independently green and independently valuable; each ends with cases in
`spec/platform/` + a baseline update + a gate step. seq359 makes handler-sessions foundational, so they
land at I2 (not late); multi-session messaging is I3.

- **I1 — Single session, single kick-off, drive-to-fixpoint (the end-to-end proof).** One `(session …)`
  (no effects yet, or only self-contained state folds), one `(kickoff …)`; grade `end-state (kv/status)`
  + `events-processed`.
  - `cdz-corpus`: parse `(platform-case …)` with `(session <alias> …)`, `(kickoff …)`, `(end-state …)`,
    `(events-processed …)` into the record stream (a new genre with its own record marker).
  - `xtask`: a `run_platform_case` path — compile `(reducer <prog>)` via the existing
    `cdz-syntax → rcdzc → component` pipeline (reuse `run_program_wasm`'s toolchain), assemble a
    `Session::genesis` with a deterministic nonce, deliver the single kick-off, drive to quiescence, grade.
  - New gate step + `spec/platform/.gate-baseline`; seed `spec/platform/01-single-session.sexp`.
  - Anchors: `xtask/src/main.rs` (`run_program_wasm` @ ~1276, `grade_trial` @ ~3790, `default_corpus_files`
    @ ~4026, `baseline_path` @ ~4093); `cdz-corpus/src/lib.rs` (`parse_case` @ ~227); `cdz-kernel/src/
    kernel.rs` (`genesis` @ ~230, `deliver` @ ~623, `kv` @ ~322, `status_snapshot` @ ~2117, `event_count`
    @ ~335, `derive_genesis_hash`); `cdz-agent-host/src/host.rs` (`AgentHost::deliver`,
    `HostedSession::genesis_with_nonce`, `SessionId`).

- **I2 — Effect-handler SESSIONS + the deterministic fixpoint drive (seq359 core).** `(serves <family>…)`
  on a session; the effect a reducer performs is DEFERRED and served by the bound handler session via the
  real `UserspaceEffectExecutor` deferral → `effect-request/<family>` inbound → handler `effect/reply` →
  `ReplyExecutor` settle. Implement the MD-SCHED deterministic FIFO breadth-first drive (D5.2) + the step
  budget (`SettleUnbounded`). Grade `expect-effects` (whole-run order-verified) + an `err` reply the caller
  compensates for. Seed `spec/platform/02-handler-sessions.sexp`. Anchors: `cdz-agent-host/src/
  {userspace_effect_exec,effect_reply,reply_exec,effect_registry}.rs`.

- **I3 — Multi-session messaging (seq358).** `Emit` (family `"message"`, target=peer alias→id) routed as a
  `message` inbound into the peer's log; `(expect-messages …)`; an `(expect-delivery-failure …)` negative
  case (emit to an unknown alias). Seed `spec/platform/03-messaging.sexp`: ping/pong, fan-out, a
  request/response whose end-state depends on the reply. Anchors: `cdz-agent-host/src/emit.rs`
  (`EmitExecutor`, family `"message"`, target=peer SessionId), `cdz-agent-host/src/async_host.rs`
  (`Inbound`, `bounce_delivery_failure`, `DELIVERY_FAILURE_FAMILY`, `MAX_HELD_INBOUND` — the reference
  behavior the runner reimplements deterministically).

- **I4 — Causality + lifecycle + recovery assertions.** Cross-session CAUSALITY: each routed message /
  effect-result carries its `cause` (the emitting dispatch id, already threaded by `EmitExecutor` /
  the reply path), so a case can assert "reporter's fold was caused by worker's emit" via the cause edge —
  the messaging analogue of the corpus's order-verified host-calls, lifted to a cross-session
  happens-before. Also `(end-state (closed …))` + `(recovers)` (the `replay ≡ recover(checkpoint+tail)`
  equivalence per session). Rides `DESIGN-session-lifecycle.md` + `DESIGN-session-log-state-decouple.md`.

- **I5 — Second-implementation differential (the portability payoff).** When a second kernel/reducer
  implementation exists, run every `spec/platform/` case against it and grade DIFFERENTIALLY against the
  oracle — no second baseline; an agreeing outcome = progress, a disagreeing outcome = the only real
  failure. Mirrors `xtask`'s `cadenza-ml-conformance-covered-subset` step. Unblocks when a second
  implementation is real.

## The gate (how it protects itself, unattended)

Adds to `cargo xtask gate`, alongside the existing `spec/semantics/` grading:
1. `cargo xtask gate` grades `spec/platform/*.sexp` and diffs against `spec/platform/.gate-baseline`
   (regression = a `pass → not-pass` flip or a vanished case; additive-only otherwise) — identical
   discipline to the compiler corpus.
2. Determinism is RUNNER-ENFORCED (D5), not conventional: the runner uses its own FIFO breadth-first drive
   (never the production `select!` loop), deterministic per-alias SessionIds (`Hash::of(salt++alias)`,
   never OS entropy), no clock/network/randomness (every effect served by a declared handler session; an
   effect for an unserved family is a Fail), and a per-case step budget (an unbounded ping-pong is a
   `SettleUnbounded` Fail, never a hang).
3. The reducers are real Cadenza compiled by the real `rcdzc` — a case that can't compile is a
   `BadArtifact` Fail, so the suite also guards reducer-authoring against compiler regressions.

## Open decisions with a chosen default (nothing here blocks the build)

- **O1 — Bytes of the grammar.** The clauses in D3 are the semantics; exact token spelling (`kickoff` vs
  `start`, `serves` vs `handles`, `expect-effects` grouping) is the vertical's to finalize WITH
  `corpus-bugfix` (who owns the reader + the flat tab-delimited record-line encoding a whole ordered
  effect/message list lowers to). Default: the spelling above. Next step: send corpus-bugfix a concrete
  sexp STRAWMAN (they asked) so they can map it to the reader Struct/Arena model + record lines.
- **O2 — Many cases per file** (as `spec/semantics/` does), grouped by feature (`NN-feature.sexp`).
- **O3 — Reducer program inline vs. `(module …)` sibling.** Reuse the corpus `(module "name" <prog>)`
  multi-file mechanism for non-trivial reducers. Default: inline for small cases, `(module …)` for larger.
- **O4 ⟨operator may ratify⟩ — Markdown-literate treatment.** Default: YES — a `.md` twin per file via
  `cdz-corpus/src/markdown.rs` (adds `session`/`kickoff`/`expect-effects`/`expect-messages`/`end-state`
  fence kinds), so platform conformance is as readable as the corpus.
- **O5 — Settle order: strict FIFO breadth-first (D5.2 default) vs. a per-session priority.** Default: FIFO
  breadth-first — the simplest total order that is obviously reproducible and matches "drain the shared
  inbox in arrival order". Whatever the choice, it is FIXED + documented so the interleaving is replay-stable.
- **O6 — `expect-effects`/`expect-messages` scope: whole-run sequence (D3/D4 default).** Default: one
  order-verified sequence over the entire run (matches the single-kickoff-to-fixpoint model; there is no
  event tape to window against). A case wanting finer granularity splits into separate cases with
  different kick-offs.

## What this is NOT

- Not a change to kernel/host production code — it consumes `cdz-kernel`/`cdz-agent-host` as-is (the
  handler-session + messaging increments coordinate with, but do not modify, the host's reply/emit
  machinery).
- Not a scripted/mock harness — there are NO canned responses and NO event tape (seq359). Effect handlers
  are real Cadenza sessions; the run is organic, driven by ONE kick-off event to a fixpoint.
- Not a replacement for the in-crate `kernel_e2e_tests` Rust units — those stay; this adds the *portable,
  Cadenza-defined, declaratively-asserted* layer on top.
- Not the compiler corpus — it's a sibling suite (`spec/platform/`) sharing the reader + gate + baseline
  machinery, with its own case genre and baselines.
