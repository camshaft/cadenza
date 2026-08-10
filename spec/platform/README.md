# Platform conformance suite

The **runtime/platform analog of the compiler corpus** (`spec/semantics/`). Where the compiler corpus
asks *"does this Cadenza program compile+run to this value/error/trap?"*, this suite asks:

> Given these Cadenza **sessions** (reducers, and — at I2 — effect-handler sessions) and **one kick-off
> event**, do the sessions — reacting organically to a fixpoint — emit these effects and these
> inter-session messages in this order, and each settle to this end-state?

It exercises `cdz-kernel` + `cdz-agent-host` (the reducer / event / KV / effect / messaging machinery)
as the system-under-test, adding **no** kernel/host production code — they are dependencies, not change
targets. Design: `implementation/design/DESIGN-platform-conformance-suite.md` (operator seq356–359);
grammar co-design: `implementation/design/platform-conformance-grammar-strawman.md`.

## How a case runs

`cargo xtask gate --target platform` grades every `spec/platform/NN-*.sexp` against
`spec/platform/.gate-baseline`, the same diff-not-count / `--save` / `--check` discipline the compiler
corpus uses (a `pass → not-pass` flip or a vanished case fails `--check`; newly-passing is reported, not
fatal). It is wired into `cargo xtask check` as a blocking gate step, so a peer's change that breaks a
platform case reds the merge gate.

The pipeline for one case:
1. `cdz-corpus` parses the `(platform-case …)` genre into flat tab-delimited record lines (a new genre
   sibling to the compiler `(case …)`; owned by `corpus-bugfix`, single-writer).
2. `xtask`'s `grade_platform_case` compiles each `(session … (reducer <prog>))` to a
   `cadenza:agent-kernel/fold` wasm component (`cdz compile --target wasm --component-name
   cadenza:agent-kernel/fold`).
3. The isolated **`cdz-session-run`** edge binary drives the constellation through the **real** kernel:
   `Session::genesis` (deterministic id `Hash::of(salt ++ alias)`) → deliver the one kick-off → drive a
   deterministic **FIFO breadth-first fixpoint** (NOT the production `select!` loop — no ordering
   guarantee) → print observed effects (whole-run order) + per-session end-state as tab lines.
4. `grade_platform_case` compares those lines to the case's `(expect-effects …)` / `(end-state …)` /
   `(events-processed …)` clauses.

Determinism (so CI is reproducible): pure-fold reducers, deterministic per-alias session ids (never OS
entropy), no clock/network/randomness, and a per-case step budget (an unbounded effect/reply ping-pong
is a graded `SettleUnbounded` failure, never a hang).

## The case grammar

```
(platform-case "<title>"
  (doc "<prose>")                                        ; optional
  (session "<alias>" (reducer <program>) (serves "<family>")…)  ; 1+; serves binds it as a handler
  (kickoff "<alias>" (inbound "<family>" (: <value> <Type>)))   ; exactly one — the only stimulus
  (expect-effects (effect (from "<a>") (family "<f>") (: <v> <T>)?)…)  ; whole-run order-verified
  (expect-messages (message (from "<a>") (to "<b>") (family "<f>") (: <v> <T>))…)
  (expect-delivery-failure (from "<a>") (to "<b>")…)
  (end-state "<alias>" (kv "<key>" (: <v> <T>))… (status <state>))  ; state: active|quiescent|stalled|closed
  (events-processed "<alias>" <n>))
```

Reducer programs use the same canonical homoiconic Cadenza s-expression the compiler corpus's
`(input …)` uses (normalized by the same reader path). Values use the corpus `(: value Type)` form so
the grader reuses value-comparison. KV values are opaque bytes at the kernel boundary; the grader
decodes the pinned `(: n IntNN)` value-form to the reducer's one-byte encoding for comparison.

## Increments

- **I1 — single session, single kick-off, drive-to-fixpoint** (LANDED). No effects; grade end-state
  (KV / status) + events-processed. See `01-single-session.sexp`.
- **I2 — effect-handler sessions** (`(serves …)` + the deterministic FIFO fixpoint drive; grade
  `expect-effects`). **Todo-witnessed** (`02-handler-sessions.sexp`): a `(serves …)` case documents the
  intended round-trip and grades `Todo` until the **binary-AST fold boundary (B2)** lands. Why: the
  handler-session round-trip needs a Cadenza reducer to emit a *register-by-string* effect family (an
  unhandled userspace family routes to the handler fallback; a handler settles via `effect/reply`), but
  the current handle-lowered fold boundary carries only the closed 6-variant `EffectKind` enum — so no
  effect a real rcdzc reducer emits reaches a handler. B2 (`apply(list<u8>) -> list<u8>`, effects as a
  cadenza-AST carrying an arbitrary family string) is the unblock. The grader Todo-gates any case with a
  `(serves …)` binding until then; when B2 lands, delete that gate and the witnesses run for real.
- **I3 — multi-session messaging** (`Emit` → peer inbound; `expect-messages` / `expect-delivery-failure`).
- **I4 — causality + lifecycle + recovery**; **I5 — second-implementation differential**.

## Layout

- `NN-feature.sexp` — the cases (only digit-led stems are graded; this README is ignored by the gate).
- `.gate-baseline` — committed per-case verdicts (`verdict\tdescription`); regenerate with
  `cargo xtask gate --target platform --save`.
- The runner lives in `implementation/seed/crates/cdz-session-run/` (an isolated workspace — it carries
  the kernel's wasmtime/tokio tree, kept out of the seed workspace + the compiler corpus gate). The grade
  path is `run_platform_case` / `grade_platform_case` in `xtask/src/main.rs`.
