# The platform conformance suite — portable, Cadenza-defined reducers + effect handlers + event/end-state assertions

Owner: TBD (a `vertical`, area `cdz-kernel` + `xtask`/`cdz-corpus`). Design by `design-platform-conformance`,
EXTENDED for multi-session interaction by `design-platform-conformance-msg`.
Status: **PROPOSAL — designed AUTONOMOUSLY, awaiting a build owner.** Operator spark via concierge
(Slack seq356, verbatim): *"I'm wondering if we should spin up a vertical for the platform conformance
suite. Like we should be able to define reducers in cadenza and effect handlers and make assertions
around events being processed and end state and everything. … really getting a good portable test
suite is worth its weight in gold. The compiler corpus suite has been incredibly valuable."* Follow-up
directive (Slack seq357): *"The design of the platform test suite needs to be autonomous."* — honored
BOTH ways: (1) this design was shaped without a live operator session — every fork below carries a
chosen default so the build is not blocked; forks the operator may want to own on return are flagged
**⟨operator may ratify⟩**; (2) the suite itself is designed to run UNATTENDED in CI — self-contained,
deterministic, machine-checkable pass/fail, exactly the way the compiler corpus runs under
`cargo xtask gate`.

Subsystem: primarily `xtask` (the runner/grader) + `cdz-corpus` (the case reader) + a new spec tree
`spec/platform/`. It exercises `cdz-kernel` (the reducer/event/KV/effect machinery) as the
system-under-test but adds NO kernel production code — the kernel is a dependency, not a change target.
Coordinated with `corpus-bugfix` (owns the `cdz-corpus` reader + `Expect` vocabulary + gate baseline
mechanics — this suite REUSES that infrastructure), `v-agent-harness` (owns the kernel seam: the
`Reducer`/`Executor` traits, the WIT `cadenza:agent-kernel` world, the `Session` drive loop),
`v-agent-harness-host` (owns `cdz-agent-host`: `UserspaceEffectExecutor`, `CompositeExecutor`
family routing, the reply/settle round-trip), and `v-effects` (language-level effect vocabulary).

> **The one-sentence thesis.** The compiler corpus asks *"does this Cadenza PROGRAM compile+run to this
> value/error/trap?"*; the platform conformance suite asks *"does this Cadenza REDUCER, driven by this
> stream of events with these effect-handler responses, emit these effects and settle to this
> end-state?"* — the same recorded-oracle, backend-agnostic, `cargo xtask gate`-run discipline, applied
> to the runtime/platform layer instead of the compiler.
>
> **The multi-session thesis (this extension, operator seq358: *"we need to be able to have more than
> one session interacting as well so we can test out messaging"*).** A conformance case may define MORE
> THAN ONE session — each its own Cadenza reducer + effect handlers — and let them INTERACT by
> messaging. The question becomes: *"do these N Cadenza reducers, driven by an event script and messaging
> each other, exchange these cross-session messages in this causal order and each settle to its own
> end-state?"* The messaging seam already exists in production (`EmitExecutor` routes a session's `Emit`
> effect to a peer's inbox as an `Inbound{family="message"}`); this extension makes that seam
> PORTABLY-TESTABLE under a deterministic, no-network, no-clock scheduler.

## Why this is worth building (and why it's cheap)

Two independent findings make this a high-leverage, low-risk vertical:

1. **The compiler corpus already models the effect boundary.** The corpus vocabulary includes
   `(host-responses (respond E.op (: v T)) …)` — canned responses to performed effects — and
   `(host-calls (call E.op arg…) …)` — the **order-verified** observed sequence of host calls a program
   makes (`cdz-corpus/src/lib.rs`, graded in `xtask/src/main.rs`). That is already "effect-handler
   responses in, assert on emitted effects out." The platform suite generalizes this from a
   single program-run to a **fold over a stream of events with durable state between them**.

2. **A platform conformance test is already a small, proven shape.** `cdz-kernel`'s
   `kernel_e2e_tests/loop_and_recovery.rs` is a working reference: build a `Session::genesis(reducer,
   nonce)`, register effect handlers on a `CompositeExecutor` (`with_effect(family, …)` /
   `with_fallback(…)`), feed events via `deliver(EventBody::Inbound{..})`, then assert on
   events-processed (`session.log()` / `event_count()`) and end-state (`session.kv().get(..)` /
   `status_snapshot()`). The suite turns that hand-written-Rust pattern into a **portable, declarative,
   Cadenza-defined case** the gate runs unattended.

So the build is mostly (a) a case format, (b) a runner that assembles a Session from a case and drives
it, and (c) a grader — riding infrastructure that already exists on both the corpus side and the kernel
side.

## The design decisions (each a fork the seed named, decided with a default)

### D1 — Format: EXTEND the corpus infrastructure with a new "session" case genre. ⟨operator may ratify⟩
**Chosen default: extend `cdz-corpus` + `cargo xtask gate` rather than stand up a parallel runner.**

Rationale: the operator's own framing — *"worth its weight in gold … the compiler corpus has been
incredibly valuable"* — is a mandate to reuse what works, not to fork it. Concretely we inherit, for
free: the tab-delimited record reader, the literate markdown surface (`cdz-corpus/src/markdown.rs`, so
a case has a `.md` twin), the `.gate-baseline` diff-not-count regression mechanics (the thing that
actually catches per-MR regressions), the `--save`/`--check` flow, the differential-target infra, and
the operator's existing mental model + tooling. A parallel format would duplicate all of that and split
the "one conformance suite" story the operator is reaching for.

The suite lives in a NEW spec tree — **`spec/platform/NN-feature.sexp`** (mirrors `spec/semantics/`) —
with its OWN baseline files (`spec/platform/.gate-baseline*`). It is a new *genre* of case within the
same reader, not a new reader. A platform case is distinguished from a semantics case by carrying a
`(session …)` block (see D3) instead of a top-level `(input …)` program.

> ⟨operator may ratify⟩ The alternative — a fully independent `spec/platform/` runner — stays open if
> the operator wants the platform suite to evolve its clause vocabulary without touching the compiler
> corpus reader. Default is REUSE; the seam (D3's grammar) is small enough to relocate later if reuse
> proves constraining.

### D2 — Runtime: the unit under test is a **Cadenza-defined reducer compiled to a wasm component**, driven through the real Rust kernel. ⟨operator may ratify⟩
**Chosen default: `Cadenza reducer → rcdzc → wasm component → cdz-kernel` (the WIT `cadenza:agent-kernel`
`apply` seam), matching the corpus's default wasm target.**

This is the only choice that honors *"define reducers in cadenza"* AND *"portable"*: the reducer is
authored in Cadenza, the same source a production agent would run, and it executes on the real kernel
drive loop — not a hand-written Rust stand-in (that would test the harness, not the platform, and
wouldn't be portable across implementations). It rides toward the `DESIGN-binary-ast-abi.md`
`apply(list<u8>) -> list<u8>` boundary, which is exactly the seam that makes a reducer definition
portable across a Rust guest and a Cadenza guest.

Effect handlers have TWO admissible realizations, and the format supports both (D4):
- **Scripted responses** (default, simplest): the case declares canned outcomes per effect family, like
  the corpus's `(host-responses …)`. The "handler" is a deterministic table, not a running session.
- **Cadenza handler sessions** (for the OUTPOST round-trip): the effect is served by *another*
  Cadenza-defined reducer session via `UserspaceEffectExecutor` deferral + `effect/reply` settle. This
  is the genuinely-novel platform behavior and is a later increment (I3).

Portability ladder (mirrors the corpus's wasm → rust → ML targets, with their own baselines /
differential grading):
- **Now (I1):** wasm reducer on the Rust kernel — the oracle.
- **Later:** a second kernel implementation (e.g. a Cadenza-ML-hosted kernel, once it exists) graded
  DIFFERENTIALLY against the Rust-kernel oracle — no second baseline, exactly how `run_program_ml`
  grades the ML compiler against the wasm oracle today. This is the "portable across implementations"
  payoff and is explicitly out of the first slices.

> ⟨operator may ratify⟩ Whether to ALSO keep a small inline-Rust-reducer fast-path for harness-level
> smoke tests. Default: NO — one path (Cadenza reducer), so the suite can never pass on a reducer that
> doesn't actually compile from Cadenza. The existing Rust `kernel_e2e_tests` remain as unit tests;
> the conformance suite does not overlap them.

### D3 — Case shape: a `(session …)` block replaces `(input …)`; events + assertions are scripted as an ordered trial stream.
A platform case, in the same s-expression surface as the corpus. Illustrative (grammar, not final
bytes):

```
(case "a counter reducer accumulates across two events and emits a log effect at threshold"
  (doc "Reducer folds Inbound `bump` events into kv `count`; at count==2 it emits one log effect.")

  ;; The system under test: a Cadenza reducer + the families its handlers serve.
  (session
    (reducer <cadenza-reducer-program>)          ;; authored in Cadenza; compiled to a component
    (handlers                                     ;; effect families this case provides responses for
      (family "log" (respond (ok)))))             ;; scripted handler: every `log` effect → Ok(())

  ;; The event script + interleaved assertions. Ordered; each (deliver ..) drives to quiescence.
  (deliver (inbound "bump" (: unit Unit)))
  (expect-effects)                                 ;; no effect emitted yet
  (deliver (inbound "bump" (: unit Unit)))
  (expect-effects (effect "log" (: "threshold" String)))   ;; order-verified emitted-effect list

  ;; Terminal assertions after the whole script.
  (end-state
    (kv "count" (: 2 Int64))                       ;; end-state KV, key -> canonical value form
    (status quiescent))                            ;; StatusSnapshot.state
  (events-processed 5))                            ;; total appended log length (Genesis + 2 Inbound + …)
```

Clause vocabulary (the NEW platform genre; each maps to an existing kernel accessor so grading is a
direct comparison, never a heuristic):

| Clause | Meaning | Kernel anchor it grades against |
|---|---|---|
| `(session (reducer <prog>) (handlers …))` | the SUT: a Cadenza reducer + scripted/handler families | `Session::genesis` + `CompositeExecutor` |
| `(deliver (inbound <family> <value>))` | append an inbound event, drive to quiescence | `Session::deliver(EventBody::Inbound{..})` |
| `(deliver (timer-fired <id>))` etc. | non-inbound stimulus (timers, results) | `EventBody::{TimerFired,EffectResult,…}` |
| `(expect-effects (effect <family> <value>)…)` | order-verified emitted-effect sequence since the last `deliver` | dispatched effects on the log (cf. corpus `(host-calls …)`) |
| `(family <name> (respond <outcome>))` | scripted handler outcome (`(ok)`,`(ok <v>)`,`(err "msg")`,`(timed-out)`) | `EffectOutcome` variants |
| `(handler-session <family> (reducer <prog>))` | a Cadenza handler SESSION (deferral round-trip) — I3 | `UserspaceEffectExecutor` + `effect/reply` |
| `(end-state (kv <key> <value>)…)` | terminal KV key→value assertions | `Session::kv().get(key)` |
| `(end-state (status <state>))` | terminal session status | `StatusSnapshot.state` (`Active/Quiescent/Stalled/Closed`) |
| `(end-state (closed (success <v>)))` | close outcome, if the session closed | `CloseOutcome` (`Success/Failure`) — I4 |
| `(events-processed <n>)` | total appended log length | `Session::event_count()` |
| `(recovers)` | replay(full) ≡ recover(checkpoint+tail) yields identical KV | `Session::replay` / `recover` — I4 |

The values everywhere use the corpus's canonical `(: value Type)` form, so the platform grader reuses
the corpus value-comparison (`grade_trial`'s value match), and the reducer/handler programs use the
same homoiconic Cadenza s-expression the corpus `(input …)` uses — so a platform case is authored,
read, and diffed by exactly the same machinery.

### D4 — Assertion surface (the initial vocabulary): emitted-effect sequence + end-state KV + session status; then close/recovery.
Ordered by increment (D5). The FLOOR that makes the suite meaningful on day one is **emitted-effect
sequence + end-state KV** (the two the operator named — *"events being processed and end state"*).
`status` is cheap (same snapshot) and included in I1. `closed`/`recovers` are I4 (they need lifecycle +
the durable-log recovery gate, which are themselves in-flight designs).

### D5 — Determinism & autonomy: the suite MUST be self-contained and reproducible with no human, no clock, no network.
This is the operator's seq357 constraint made concrete — the invariants the runner enforces so a case
can never be flaky:

1. **No real effects escape.** Every effect family a reducer can emit is served by a scripted
   `(respond …)` table or a Cadenza `(handler-session …)`; there is no live executor. An emitted effect
   for a family the case did NOT declare is a **case failure** (surfaced, not silently dropped —
   mirrors `CompositeExecutor`'s observable-Err-on-unroutable, §9d anti-stuck).
2. **No wall clock, no randomness.** Timers advance only via explicit `(deliver (timer-fired …))`;
   `now`-family effects are answered from the scripted table. The kernel is already replay-deterministic
   (its `replay_determinism` test proves live-kv ≡ replayed-kv); the suite inherits that.
3. **Fuel-bounded.** Reducer folds run under the kernel's fuel budget; exhaustion is a recorded
   `FoldFailed`, gradeable as such, never a hang.
4. **Machine-checkable pass/fail via the baseline diff.** Like the corpus, the verdict is a per-case
   `Pass/Todo/Fail` rolled into `spec/platform/.gate-baseline`; a `pass → not-pass` flip or a vanished
   case is a regression that fails `gate --check`. Newly-passing cases are reported, not fatal. This is
   what lets the vertical grow the suite unattended without renegotiating a pass-count each MR.

## Increments (top-to-bottom, the way a vertical lands them)

Each increment is independently green and independently valuable; each ends with cases in
`spec/platform/` + a baseline update + a gate step.

- **I1 — Single-session, scripted-effect conformance (the end-to-end proof).**
  - `cdz-corpus`: parse the `(session …)` / `(deliver …)` / `(expect-effects …)` / `(end-state …)` /
    `(events-processed …)` clauses into the record stream (a `Session`-genre record).
  - `xtask`: a `run_session_case` path — compile the `(reducer <prog>)` via the existing
    `cdz-syntax → rcdzc → component` pipeline (reuse `run_program_wasm`'s toolchain), assemble a
    `Session::genesis` + a `CompositeExecutor` from the scripted `(family …)` table, replay the
    `(deliver …)` script, and grade `expect-effects` / `end-state (kv/status)` / `events-processed`.
  - New gate step + `spec/platform/.gate-baseline`; seed `spec/platform/01-reducer-fold.sexp` with the
    counter example above plus 3–5 sibling cases (fold accumulation, an effect emitted, an `err`
    response handled). Ports the `loop_and_recovery` pattern into a portable case.
  - Anchors: `xtask/src/main.rs` (`run_program_wasm` @ ~1276, `grade_trial` @ ~3790, `default_corpus_files`
    @ ~4026, `baseline_path` @ ~4093); `cdz-corpus/src/lib.rs` (`parse_case` @ ~227, `Expect` @ ~95);
    `cdz-kernel/src/kernel.rs` (`genesis` @ ~230, `deliver` @ ~623, `kv` @ ~322, `status_snapshot` @
    ~2117, `event_count` @ ~335); `cdz-kernel/src/executor.rs` (`CompositeExecutor` @ ~73).

- **I2 — Non-inbound stimuli + richer effect assertions.** `(deliver (timer-fired …))`,
  `(deliver (effect-result …))`; effect payload matching on `target` + payload; `(err …)`/`(timed-out)`
  responses and asserting the reducer's compensating behavior. Grows `spec/platform/02-*`.

- **I3 — Cadenza handler SESSIONS (the OUTPOST deferral round-trip).** `(handler-session <family>
  (reducer <prog>))`: the effect is served by a *second* Cadenza reducer via `UserspaceEffectExecutor`
  (deferral → `effect-request/<family>` Inbound into the handler → handler emits `effect/reply` →
  `ReplyExecutor` settles onto the caller's open `EffectId`). This is the platform behavior that has no
  compiler-corpus analogue and is the vertical's highest-value target. Coordinate with
  `v-agent-harness-host` (owns that machinery). Anchors: `cdz-agent-host/src/{userspace_effect_exec,
  effect_reply,reply_exec,effect_registry}.rs`.

- **I4 — Lifecycle + recovery assertions.** `(end-state (closed …))` and `(recovers)`: close outcome
  grading and the `replay ≡ recover(checkpoint+tail)` equivalence check per case. Rides
  `DESIGN-session-lifecycle.md` + `DESIGN-session-log-state-decouple.md` as they land; gated on them.

- **I6 — Multi-session interaction / messaging (operator seq358).** N interacting sessions that message
  each other; deterministic breadth-first settle; `expect-messages` + per-alias `end-state`; then
  causality + composition with handler-sessions. FULLY SPECIFIED in the "MULTI-SESSION extension" section
  below (MD1–MD5, sub-increments I6a/I6b/I6c). Sequenced after I1–I2, independent of I3–I5.

- **I5 — Second-implementation differential (the portability payoff).** When a second kernel/reducer
  implementation exists (a Cadenza-ML-hosted kernel, or the binary-AST guest as a distinct backend),
  run every `spec/platform/` case against it and grade DIFFERENTIALLY against the Rust-kernel oracle —
  no second baseline, an agreeing outcome = progress, a disagreeing outcome = the only real failure.
  Mirrors `xtask`'s `cadenza-ml-conformance-covered-subset` step. Explicitly future; unblocks when a
  second implementation is real.

## MULTI-SESSION extension — inter-session messaging conformance (operator seq358)

This extends the SINGLE-session suite above (I1 = one reducer; I5 = second-implementation differential)
with a genuinely new axis: a case defining **N interacting sessions** that message each other. It builds
ON the format and runner above — a multi-session case is the same `spec/platform/` genre with more than
one `(session …)` block — and lands as its own increment (I6, sequenced after the single-session I1–I2
prove the runner; independent of I3–I5).

### The messaging seam already exists — the extension makes it PORTABLY testable
The runtime already implements cross-session messaging (verified in `cdz-agent-host/src/emit.rs` +
`async_host.rs`), and the design reuses it verbatim rather than inventing a parallel one:

- **Addressing + send.** A reducer in session A performs an `Emit` effect (`EffectKind::Emit` /
  `effect_ct::EMIT`, family `"message"`) whose `target` is the PEER's `SessionId` (opaque bytes read as
  UTF-8, `EffectRequest::target_str`) and whose `payload` is the message (opaque — the reducer defines
  the schema). The kernel authorizes then dispatches to `EmitExecutor`, which routes the signal to the
  host's shared `Inbox`.
- **Delivery.** The routed signal becomes an `EventBody::Inbound { content_type.family = "message",
  payload }` delivered into the TARGET session's log; the peer reducer folds it with the ordinary inbound
  pattern. Send is FIRE-AND-FORGET — the executor returns `Ok(None)` (a unit ack that the signal was
  enqueued) the moment the enqueue succeeds; the sender does NOT await the peer's processing.
- **Undeliverable = bounce, not silent drop.** If `target` is gone/terminated, the loop bounces a
  `delivery-failure`-family `Inbound` back to the sender's `reply_to` (`bounce_delivery_failure`), so an
  undeliverable emit is observable to the sender's reducer. The suite asserts on this too (a negative
  messaging case: emit to an unknown/closed peer → sender folds a `delivery-failure`).

### MD1 — Addressing: cases name sessions by a stable local ALIAS; the runner binds alias → deterministic SessionId.
A case must address peers WITHOUT hardcoding a genesis-hash-hex (which is content-derived and would churn
on any reducer edit). So a multi-session case gives each session a short **alias** (`"alice"`, `"bob"`);
the runner assigns each a deterministic `SessionId` and threads the mapping into the reducers so a sender
can name its target.

- **Deterministic ids.** `SessionId` IS the session's genesis `Hash`, and the host exposes
  `HostedSession::genesis_with_nonce` + `Session::derive_genesis_hash(reducer, nonce, parent)` — a
  caller-SUPPLIED nonce. The runner derives each session's nonce as `Hash::of(case_salt ++ alias)`
  (NOT OS entropy — that would be non-reproducible), so a case's session ids are a pure function of the
  case, identical every run and in CI. This is the determinism analogue of the corpus's fixed inputs.
- **How a reducer learns a peer's id.** The runner delivers each peer's `(alias, SessionId-hex)` bindings
  as an early genesis-context `Inbound` (family `platform/peers`, one per case, folded into session KV
  under a well-known key) BEFORE the event script runs — mirroring the host's existing genesis-setup
  seed path (`genesis_ct::CONTEXT`, delivered as ordinary early inbound events). A reducer that emits to
  `"bob"` reads `kv["peers"]["bob"]` for the target id. The case's `(emit …)` clauses (below) address by
  ALIAS; the runner rewrites alias → id at the boundary, so the case author never sees a hash.

> ⟨operator may ratify⟩ Alternative addressing: a flat well-known name registry (send to `"bob"` directly,
> host resolves). Default is alias→id-in-KV because it uses the EXISTING genesis-context seed + Emit
> target-is-SessionId contract with no new kernel/host surface. If the OUTPOST federation later adds a
> name-directory effect (`session-directory` design), the suite adopts it then.

### MD2 — Determinism: a case-driven DETERMINISTIC scheduler, NOT the production `select!` loop.
This is the load-bearing decision for CI reproducibility. The production multi-session loop
(`AsyncAgentHost::run`) multiplexes sessions with tokio `select!`, which by its own doc gives **NO
ordering/fairness guarantee** across the inbound channel + timers — perfect for a live host, fatal for a
reproducible test. So the conformance runner does NOT use `AsyncAgentHost::run`. Instead it drives the
lower, deterministic primitive directly:

- The runner holds the `AgentHost` registry (N `HostedSession`s) and its OWN in-memory message queue in
  place of the mpsc `Inbox`. It calls the synchronous-per-step `AgentHost::deliver(session, body, cause)`
  itself, one delivery at a time, and DRAINS routed emits from its own queue in **strict FIFO** order.
- **The scheduling rule (deterministic, total, documented in the case):** process the case's explicit
  `(deliver …)`/`(emit …)` script in written order; after each delivery, drive that session to
  quiescence; any `message` emits it produced are appended to the queue in emission order; then drain the
  queue FIFO, each drained message delivered to its target and driven to quiescence (which may enqueue
  more) — a deterministic breadth-first settle. The case is DONE when the script is exhausted AND the
  queue is empty (a quiescent global state). This is a fixed, replay-stable interleaving with no
  wall-clock and no `select!` nondeterminism.
- **Timers stay explicit** (as in single-session D5): no timer fires except via an explicit
  `(deliver <alias> (timer-fired …))` step, so cross-session timing is authored, never raced.
- **Cycle/liveness bound:** the global settle runs under a per-case message-count budget (like the fuel
  bound on folds); exceeding it (an unbounded message ping-pong) is a recorded `SettleUnbounded` Fail,
  never a hang. So a messaging livelock is a gradeable failure, not a stuck CI job.

### MD3 — Case shape: multiple `(session <alias> …)` blocks + alias-addressed `(emit …)` + cross-session assertions.
The single-session `(session …)` block (D3) gains an alias; a multi-session case has more than one, plus
new clauses to address a peer and to assert on the cross-session flow. Illustrative:

```
(case "two sessions exchange a ping/pong and each settles to a seen-count of 1"
  (doc "alice emits `ping` to bob; bob folds it, increments seen, emits `pong` back; alice folds pong.")

  (session "alice"
    (reducer <alice-reducer-program>)
    (handlers (family "message" (respond (ok)))))   ;; the `message` family (Emit ack) → Ok(())
  (session "bob"
    (reducer <bob-reducer-program>)
    (handlers (family "message" (respond (ok)))))

  ;; Stimulus: an external inbound kicks alice, whose reducer emits to bob.
  (deliver "alice" (inbound "start" (: unit Unit)))

  ;; Cross-session message-flow assertion: the order-verified sequence of DELIVERED inter-session
  ;; messages across the whole settle (sender-alias, target-alias, family, payload). This is the new
  ;; multi-session oracle — the analogue of single-session `expect-effects`, but for routed messages.
  (expect-messages
    (message (from "alice") (to "bob")   (family "message") (: "ping" String))
    (message (from "bob")   (to "alice") (family "message") (: "pong" String)))

  ;; Per-session end-state, keyed by alias.
  (end-state "alice" (kv "seen" (: 1 Int64)) (status quiescent))
  (end-state "bob"   (kv "seen" (: 1 Int64)) (status quiescent)))
```

New / extended clauses (each maps to an existing kernel/host accessor so grading stays a direct compare):

| Clause | Meaning | Anchor it grades against |
|---|---|---|
| `(session <alias> (reducer <prog>) (handlers …))` | one SUT session, named by a case-local alias | `HostedSession::genesis_with_nonce` (nonce = `Hash::of(salt++alias)`) |
| `(deliver <alias> <body>)` | deliver a stimulus to a NAMED session, drive to quiescence | `AgentHost::deliver(id_of(alias), body, cause)` |
| `(emit <alias> (message <target-alias> <value>))` | (optional authored stimulus) inject a message as if from a session — mostly reducers `Emit` themselves | routes through the runner's queue exactly like an `EmitExecutor` output |
| `(expect-messages (message (from <a>) (to <b>) (family <f>) <value>)…)` | order-verified sequence of DELIVERED inter-session messages over the settle | routed `Inbound{family="message"}` deliveries the runner observed on its queue |
| `(expect-delivery-failure (from <a>) (to <alias-or-id>) …)` | a bounce: emit to a gone/unknown peer → sender folds `delivery-failure` | `bounce_delivery_failure` / `DELIVERY_FAILURE_FAMILY` |
| `(end-state <alias> (kv …)(status …))` | per-session terminal assertions, keyed by alias | `session(alias).kv()/status_snapshot()` |

The single-session case (D3) is the degenerate `N=1` shape: one unnamed (or single-alias) session, no
`(expect-messages …)`. The runner picks the multi-session path iff the case has >1 `(session …)` block
OR any `expect-messages`/`emit`/aliased clause — so the two genres share one reader + grader.

### MD4 — Increment I6, layered on the single-session runner.
Sequenced AFTER I1–I2 (which prove the single-session runner + the deterministic drive), independent of
the I3–I5 (handler-session / lifecycle / differential) axes. Independently green + valuable.

- **I6a — Two-session messaging, scripted handlers (the messaging end-to-end proof).**
  - `cdz-corpus`: parse aliased `(session <alias> …)`, `(deliver <alias> …)`, `(expect-messages …)`,
    per-alias `(end-state <alias> …)` into the record (a multi-session Session-genre record).
  - `xtask`: extend `run_session_case` with a `run_multi_session_case` path — build N `HostedSession`s
    with deterministic per-alias nonces, seed each with its `platform/peers` alias→id context, run the
    MD2 deterministic breadth-first settle over the runner's own FIFO queue (NOT `AsyncAgentHost::run`),
    and grade `expect-messages` (order-verified routed-message sequence) + per-alias `end-state`.
  - Seed `spec/platform/10-two-session-messaging.sexp`: the ping/pong above, a fan-out (one → two peers),
    a request/response where the requester's end-state depends on the reply, and an `expect-delivery-failure`
    negative case (emit to an unknown alias). Baseline into `spec/platform/.gate-baseline`.
  - Anchors: `cdz-agent-host/src/emit.rs` (`EmitExecutor`, family `"message"`, target=peer SessionId),
    `cdz-agent-host/src/async_host.rs` (`Inbound`, `bounce_delivery_failure`, `DELIVERY_FAILURE_FAMILY`,
    `MAX_HELD_INBOUND` backpressure — the reference behavior the runner reimplements deterministically),
    `cdz-agent-host/src/host.rs` (`AgentHost::deliver`, `HostedSession::genesis_with_nonce`, `SessionId`),
    `cdz-kernel/src/kernel.rs` (`Session::derive_genesis_hash`).

- **I6b — N-session + causality assertions.** More than two sessions; assert cross-session CAUSALITY, not
  just the flat delivered-message sequence: each routed message carries its `cause` (the emitting dispatch
  id, already threaded by `EmitExecutor`), so a case can assert "bob's `pong` was caused by alice's
  `ping`" via the cause edge on the peer's log — the messaging analogue of the corpus's order-verified
  host-calls, lifted to a cross-session happens-before. Grows `spec/platform/11-*`.

- **I6c — Messaging × handler-sessions (composes with I3).** A message TARGET can itself be a
  `(handler-session …)` reducer (I3): the OUTPOST round-trip and peer messaging are the same routing
  substrate (both `Inbound` deliveries into a peer's log), so once I3 + I6a land, a case can mix
  reducer-to-reducer messages with effect-deferral-to-a-handler-session. This is where the platform suite
  most directly exercises the `v-agent-harness-host` federation (the OUTPOST ws-transport host routes
  exactly these cross-session `Inbound`s over a wire; the suite is its in-process, deterministic oracle).

### MD5 — Relation to the OUTPOST / federation (`v-agent-harness-host`).
The OUTPOST (`design-the-outpost-host-websocket-federation-node`) is the PRODUCTION cross-node router:
it carries the same `Emit`→peer-`Inbound` messaging over a WebSocket wire between federation nodes. This
suite is the **in-process, deterministic conformance oracle** for that behavior: the message SEMANTICS
(family `"message"`, target=SessionId, fire-and-forget, delivery-failure bounce, cause provenance) are
identical whether routed in-process or over ws-transport, so a reducer that passes a `spec/platform/`
messaging case behaves identically when federated. The vertical should coordinate I6c with
`v-agent-harness-host` so the OUTPOST's wire-level tests and this suite's declarative cases assert the
SAME contract from the two ends (wire vs. semantics). The suite adds no host/kernel production code
(consistent with "What this is NOT" below); it consumes the emit/routing machinery as-is.

## The gate (how it protects itself, unattended)

Adds to `cargo xtask gate`, alongside the existing `spec/semantics/` grading:
1. `cargo xtask gate` grades `spec/platform/*.sexp` and diffs against `spec/platform/.gate-baseline`
   (regression = a `pass → not-pass` flip or a vanished case; additive-only otherwise) — identical
   discipline to the compiler corpus.
2. A platform case that emits an undeclared effect family, hangs on fuel, or reads a clock/network is a
   **Fail** by construction (D5) — the determinism invariants are enforced by the runner, not by
   convention.
3. The reducer/handler programs are real Cadenza compiled by the real `rcdzc` — a case that can't
   compile is a `BadArtifact` Fail, so the suite also guards reducer-authoring against compiler
   regressions.
4. **Multi-session determinism is runner-enforced (MD2), not conventional.** A multi-session case runs
   under the runner's OWN deterministic FIFO scheduler (never the production `select!` loop), deterministic
   per-alias SessionIds (`Hash::of(salt++alias)`, never OS entropy), explicit-only timers, and a per-case
   message-count budget — so a reproducible messaging interleaving is a structural property, and an
   unbounded message ping-pong is a `SettleUnbounded` Fail, never a hung CI job.

## Open decisions with a chosen default (nothing here blocks the build)

- **O1 — Bytes of the `(session …)` grammar.** The clauses in D3 are the semantics; exact token spelling
  (`expect-effects` vs `emits`, `end-state` grouping) is the vertical's to finalize with `corpus-bugfix`
  (who owns the reader). Default: the spelling above.
- **O2 — One case = one file, or many cases per file (as `spec/semantics/` does)?** Default: many per
  file, grouped by feature (`NN-feature.sexp`), matching the corpus.
- **O3 — Reducer program inline vs. `(module …)` sibling.** For non-trivial reducers, reuse the corpus
  `(module "name" <prog>)` multi-file mechanism. Default: inline for I1, `(module …)` when a case needs
  a library.
- **O4 ⟨operator may ratify⟩ — Does the platform suite share `spec/`'s markdown-literate treatment?**
  Default: YES — a `.md` twin per file via `cdz-corpus/src/markdown.rs` (adds `session`/`deliver`/
  `expect-effects`/`end-state`/`expect-messages` fence kinds), so platform conformance is as readable as
  the corpus.
- **O5 — Multi-session settle order: strict FIFO breadth-first (MD2 default) vs. a per-session priority.**
  Default: FIFO breadth-first — the simplest total order that is obviously reproducible and matches "drain
  the shared inbox in arrival order". A priority/round-robin discipline is deferred unless a real case
  needs it; whatever the choice, it is FIXED and documented so the interleaving is replay-stable.
- **O6 — Peer-address surface in the reducer: `platform/peers` KV seed (MD1 default) vs. a name-directory
  effect.** Default: alias→id delivered as a genesis-context `Inbound` folded into KV, reusing the existing
  seed path with no new surface. Revisit if/when the `session-directory` design lands a lookup effect.
- **O7 — `expect-messages` scope: whole-settle sequence (MD3 default) vs. per-`deliver` windows.** Default:
  one order-verified sequence over the entire settle (simplest, matches the breadth-first drain). A case
  needing finer granularity can split into multiple `(deliver …)` steps with interleaved `expect-messages`,
  exactly as single-session `expect-effects` interleaves with `deliver`.

## What this is NOT

- Not a change to kernel/host production code — it consumes `cdz-kernel`/`cdz-agent-host` as-is (I3
  coordinates with, but does not modify, the host's reply machinery).
- Not a replacement for the in-crate `kernel_e2e_tests` Rust units — those stay; this adds the
  *portable, Cadenza-defined, declaratively-asserted* layer on top.
- Not the compiler corpus — it's a sibling suite (`spec/platform/`) sharing the reader + gate + baseline
  machinery, with its own case genre and baselines.
