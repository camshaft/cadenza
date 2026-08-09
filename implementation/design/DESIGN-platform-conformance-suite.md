# The platform conformance suite — portable, Cadenza-defined reducers + effect handlers + event/end-state assertions

Owner: TBD (a `vertical`, area `cdz-kernel` + `xtask`/`cdz-corpus`). Design by `design-platform-conformance`.
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

- **I5 — Second-implementation differential (the portability payoff).** When a second kernel/reducer
  implementation exists (a Cadenza-ML-hosted kernel, or the binary-AST guest as a distinct backend),
  run every `spec/platform/` case against it and grade DIFFERENTIALLY against the Rust-kernel oracle —
  no second baseline, an agreeing outcome = progress, a disagreeing outcome = the only real failure.
  Mirrors `xtask`'s `cadenza-ml-conformance-covered-subset` step. Explicitly future; unblocks when a
  second implementation is real.

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
  `expect-effects`/`end-state` fence kinds), so platform conformance is as readable as the corpus.

## What this is NOT

- Not a change to kernel/host production code — it consumes `cdz-kernel`/`cdz-agent-host` as-is (I3
  coordinates with, but does not modify, the host's reply machinery).
- Not a replacement for the in-crate `kernel_e2e_tests` Rust units — those stay; this adds the
  *portable, Cadenza-defined, declaratively-asserted* layer on top.
- Not the compiler corpus — it's a sibling suite (`spec/platform/`) sharing the reader + gate + baseline
  machinery, with its own case genre and baselines.
