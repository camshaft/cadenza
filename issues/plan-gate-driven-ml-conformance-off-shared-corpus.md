# Plan: gate-driven ML-compiler conformance off the SHARED corpus (operator directive, 2026-07-17)

## CORPUS-SHAPE FINDING (2026-07-17, surveyed spec/semantics — reshapes the front-end)

The corpus `(input …)` is the CANONICAL HOMOICONIC S-EXPR (PREFIX), not surface source and not my infix
`Tok` lists. Real examples:
- `01-literals`: `(input 42)`, `(input true)`, `(input 3.5)`, `(input "hello")` — output `(: <val> <Type>)`.
- `02-binding-and-control`: `(input (let ((x 10)) x))` → `(: 10 Int64)`; `(input (let ((x 1)) (let ((x 2)) x)))`
  → `(: 2 Int64)`; `(input (if false 1 2))` → `(: 2 Int64)`. ~37 let/if cases outputting `(: N Int64)`.
IMPLICATION for the front-end behind `cdz run-ml`: it must read S-EXPR PREFIX (`(let ((x V)) B)`, `(if C T E)`,
`(op a b)` PREFIX, int/bool literals) — NOT infix. So it should feed my `resolve/infer/lower/eval` columns
DIRECTLY from an s-expr reader, BYPASSING the infix `parse-db` (which is for hand-built infix `Tok` lists and
is the wrong front for corpus input). i.e. the reader = s-expr text → `Node` arena (the shape resolve-db
consumes), NOT s-expr → `Tok` → parse-db. This narrows the "source→Tok" framing: it's "s-expr→arena". The
supported first slice = int/bool literals + `let`/`if` + `(+ - * / % < == > <= >= !=)` PREFIX over ints →
a real handful of `01`/`02` cases flip to `value` immediately. `run-ml` prints `value (: N Int64)` /
`value (: true Bool)` to match the corpus output value-form. (Feeds directly into the A/B/C choice: A's
"gate-side Rust Tok-encoder" is doubly wrong now — corpus is prefix s-expr, my Tok pipeline is infix; C's thin
s-expr→arena reader on the ML side is the clean fit.)


**Directive (operator, verbatim):** "Why is the compiler ml hardcoding corpus inputs as tests. That's the
wrong approach. It needs to include it as part of the xtask gate." → the Cadenza-in-Cadenza (ML) compiler's
conformance must be driven by the SHARED corpus (`spec/semantics/*.sexp`) via `cargo xtask gate`, the same
corpus rcdzc is gated against — NOT a hand-maintained set of `c-*`/`conformance-*` case-builders inside
`conformance-db.cdz` (duplicative, drifts, and caused today's fleet-blocking trunk-red).

## Current state (what exists)

- The ML pipeline (`parse-db → resolve-db → infer-db → lower-db → eval-db`) consumes a **`List(Tok)`** —
  hand-built token lists — NOT `.sexp` source, NOT Cadenza surface syntax. `conformance-db.cdz` hand-encodes
  ~47 programs as `Tok` lists with expected outcomes and asserts them as in-file `@test`s. This is the
  pattern the operator is rejecting.
- The gate (`xtask/src/main.rs`): `read_corpus` parses each `spec/semantics/*.sexp` case (`(case … (input …)
  (output (: <val> <Type>)))`); `gate_one_case`/`run_program` dispatch per `GateTarget` (Wasm / Rust /
  RustAsync) — compile+run the corpus program on that backend, compare to the expected output. rcdzc is the
  `Rust` target. There is NO `cadenza-ml` target yet.

## The gap (why this is a real design fork, not a mechanical rewrite)

1. **INPUT FORMAT.** The gate has each case's program as parsed s-expr AST / surface source. My ML compiler's
   front door is `List(Tok)` over a HAND-ROLLED token type (`parse-db.Tok`), with no lexer from source and no
   `.sexp` reader. To feed corpus programs to the ML compiler, SOMETHING must turn corpus source → my `Tok`
   list (or my compiler must grow a real front-end).
2. **COVERAGE.** The 28 corpus files span the WHOLE language (literals, binding/control, equality,
   capabilities, compound types, numeric model, type system, functions, bytes, …). My ML compiler handles a
   TINY integer/bool-arithmetic subset (`+ - * / %`, `< == > <= >= !=`, `let`, `if`, unary minus). It would
   DECLINE the vast majority — correctly, but the gate must treat "ML compiler doesn't support this yet" as a
   graceful not-yet-covered, not a red.

## Proposed architecture (mirrors rcdzc's target model)

Add a `cadenza-ml` `GateTarget` to `cargo xtask gate` (v-fleet-tooling owns `xtask`; I own the ML compiler +
the entry it calls). For each corpus case the gate:
  (a) feeds the case's program to the ML compiler,
  (b) collects the ML compiler's verdict (ran-to-value / declined / error),
  (c) compares to the corpus's expected `(output …)`,
  (d) reports **ML-conformance X/N** (X supported-and-correct out of N total), the not-yet-supported ones
      tallied separately (a "supports M/N" coverage number that CLIMBS as the language widens — the
      start-low-and-climb scoreboard the operator wanted, now off the REAL corpus).
Then DELETE the hand-encoded `c-*`/`conformance-*` case-builders from `conformance-db.cdz` (keep only the
engine: `Case`/`Expect`/`case-passes`/`run`, reused by the gate seam).

## THE DESIGN FORK (routing to concierge → operator)

How does a corpus case's program reach the ML compiler's `Tok` list? Three options:

- **(A) Gate-side adapter (Rust).** `xtask` converts each corpus case's parsed s-expr program → the ML
  compiler's `Tok` sequence (a small Rust encoder over `parse-db.Tok`'s known op-codes), invokes the ML
  compiler (via `cdz` running an entry in `eval-db`, or a thin harness), reads back the value. PRO: no ML
  front-end work; reuses the existing `Tok` door. CON: the `Tok` encoder is a second (Rust) mirror of the
  grammar — some duplication, though far less than 47 hand-cases, and it lives in the gate not the corpus.
- **(B) ML compiler grows a real `.sexp`/source front-end.** Add a lexer (source → `Tok`) and an s-expr
  reader to the ML compiler so it consumes corpus `input` directly. PRO: the truest self-hosting mirror (the
  ML compiler reads the same text rcdzc does); no gate-side grammar mirror. CON: significant front-end work
  (lexer + reader in Cadenza) BEFORE any conformance number; larger blast radius.
- **(C) A `cdz` subcommand the gate shells out to.** `cdz run-ml <program-source>` (a new mode wiring the ML
  pipeline to a source string); the gate feeds corpus source and diffs stdout. PRO: clean process boundary,
  gate stays thin. CON: still needs (B)'s source→Tok front-end inside that subcommand.

**My lean: (A) first** — it delivers a REAL gate-driven conformance number off the shared corpus THIS
increment with the least new surface (a bounded Rust `Tok`-encoder for the integer/bool subset the ML
compiler supports; every other corpus case = "not yet supported", counted, not red). (B) is the eventual
self-hosting endpoint (the ML compiler reading real source) and can replace the gate-side encoder later — it
becomes a widening milestone, not a prerequisite. Coordinate the `GateTarget`/harness wiring with
v-fleet-tooling regardless of A/B/C.

### FORK-SHARPENING FINDING (2026-07-17, after reading `xtask/src/main.rs`) — LEAN REVISED to (C)

The gate hands each target the program as an **s-expr SOURCE STRING**: `run_program(program: &str, …)` →
`run_program_rust` pipes `cdz convert | cdz compile - --target rust`; wasm the analogous source path. There
is NO pre-parsed-AST handoff — a backend receives SOURCE and its front-end reads it. So option **(A) is
weaker than it first looked**: a gate-side Rust "corpus→`Tok`" encoder would have to **re-parse the corpus
s-expr source IN RUST** (a duplicate reader + grammar mirror), nearly as much work as a real front-end.
**REVISED LEAN → (C):** a `cdz run-ml <source>` subcommand backed by a THIN s-expr-source→`Tok` reader
feeding the existing ML pipeline; the gate pipes corpus source to it exactly as it does `cdz compile` for
rcdzc — symmetric, no gate-side grammar mirror, truest self-hosting. Scope to the supported subset
(unsupported program → clean decline, counted coverage-not-yet). The thin reader is the ML-side work I own;
v-fleet-tooling wires the `cadenza-ml` GateTarget to shell `cdz run-ml`. (A) only wins if we DON'T re-parse
source — but the gate contract says we must. Awaiting operator ruling with this in hand.

## Increment plan (once fork resolved)

1. v-fleet-tooling + me: add the `cadenza-ml` `GateTarget` skeleton + the feed/collect seam (per chosen fork).
2. Me: the ML-compiler entry the gate invokes (subset-runner: source/Tok → run → value|decline).
3. Gate reports `cadenza-ml: X/N` (supported-correct) + `M not-yet-supported`; wire into `check` as a
   REPORTED (non-baseline) line so it never reds the fleet (a widening scoreboard, not a pass/fail gate).
4. DELETE the hand-encoded case-builders from `conformance-db.cdz` (retain the engine).
5. Widen the ML compiler case-by-case; the shared-corpus number climbs — no more hand-encoded drift, no more
   hardcoded-case trunk-reds.

## Open question for operator (via concierge)

Which fork — (A) gate-side Rust `Tok`-encoder for the supported subset [my lean, fastest to a real number],
(B) ML compiler grows a source/`.sexp` front-end [truest self-hosting, more work first], or (C) a `cdz
run-ml` subcommand? And: report `cadenza-ml` conformance as a NON-baseline reported line (climbing coverage),
not a hard gate, so partial support never reds the fleet — confirm?
