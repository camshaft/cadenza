# Plan: gate-driven ML-compiler conformance off the SHARED corpus (operator directive, 2026-07-17)

## THE run_run_ml WIRE — mechanism PINNED (2026-07-17, ready to build on a quiet box + landed sread)

End-to-end proven: `sread-eval.run-src(s) = read-source(s)→eval-tree` runs a program FROM SOURCE →
`Option(Int64)` (milestone `(let ((x 2)) (+ (* x 3) 1))`→7). `Option(Int64)` CROSSES the boundary; `cdz run`
renders `(: (Some 42) (Option Int64))` / `(: (None unit) …)`. So `run_run_ml` (Rust, cdz/src/main.rs) does:
1. read the corpus program SOURCE (file/stdin) — already done in the stub.
2. escape it as a Cadenza STRING LITERAL, generate a driver:
     `import { run-src } from "sread-eval"`
     `def main() = run-src("<ESCAPED-SOURCE>")`
     `export { main }`
   (source is a compile-time literal — NO String boundary crossing, the constraint that ruled out an arg.)
3. IMPORT RESOLUTION: `closure::load` resolves imports RELATIVE TO THE ENTRY FILE'S DIR (`entry.parent()`;
   confirmed closure.rs:90). There is NO `--search-path` flag. So the driver MUST be written INTO
   `implementation/compiler-ml/src/` (alongside sread-eval + the pipeline modules) so `import "sread-eval"`
   resolves. Write a temp driver there (unique name, e.g. `zz-run-ml-driver.cdz`), compile+run, then DELETE it.
   ⚠ temp file in the tracked src dir — must clean up even on error; consider a `.gitignore`d name / guard.
4. compile+run: either IN-PROCESS (like `cdz test`/`cdz run` — cdz-run is linked in) or shell `cdz compile |
   cdz run`. In-process is cleaner (no PATH dep); the run yields the rendered `Option`.
5. parse the render: `(: (Some N) (Option Int64))` → print `value N` (BARE scalar N, matching rcdzc's
   Ran::Value render for the differential); `(: (None …) …)` → print `declined`; anything else / trap →
   `error <msg>` (or `declined` for out-of-subset). Exit 0 (verdict), per contract.
BLOCKED-ON: sread + sread-eval must be ON TRUNK first (the driver imports them; pr-sync re-gate compiles the
driver). All 4 (sread slices 2-4 + sread-eval) are pending. Build the wire once they land + box is quiet
(heavy: full `cdz` crate compile + a driver compile-per-case).

## INVOCATION ARCHITECTURE (settled 2026-07-17: SOURCE-IN, per operator "exact same way as rcdzc")

`cdz run-ml` STUB LANDED on trunk (`c009ad30c`). Now the front-end behind it. The operator: the ML compiler
is "compiled and invoked the EXACT SAME WAY rcdzc does" — and rcdzc takes **program SOURCE IN** (`cdz convert
| cdz compile -`). So `run-ml` must feed the ML compiler a **program source string** and the ML compiler must
READ it — NOT the Rust side pre-parsing to my `Tok`/arena. Concretely (studied `cdz run`/`cdz-run` RunArgs):
- `run-ml` reads the corpus program SOURCE (file/stdin) — already does (stub).
- It hands that source STRING to the Cadenza-written ML pipeline. Mechanism candidates:
  - ❌ **(ii-a) `run-source(program: String)` — RULED OUT (spiked 2026-07-17).** A `.cdz` entry CANNOT take a
    `String` arg across the component boundary: `cdz compile` errors "type `String` has no component boundary
    representation (only the aliased integer widths 8/16/32/64 cross the boundary)". So the program source
    cannot be handed to the ML compiler as a String arg. (Confirms the memory trap that String doesn't cross
    the host/component boundary — applies to ARG-IN too, not just RETURN.)
  - ✅ **(ii-c) BYTE-LIST source-in — the viable path.** `Int64`/width-aliased ints DO cross the boundary, so
    a `List` of byte codes can. `run-ml` (Rust) reads the corpus source, converts to a byte list, passes it
    via `--arg` (or the driver embeds it), and the ML compiler's reader consumes **bytes** — which `strlex`
    ALREADY does (`String.to-bytes` / `char-at` over bytes). This IS "source in" (the ML compiler reads the
    raw program bytes itself, self-hosting-true) and crosses the boundary legally. ⚠ NEXT VERIFY (small spike,
    quiet box): does a `List(Int64)`/`List(UInt8)` arg cross via `cdz run --arg`? If `--arg` can't express a
    list, fall to (ii-d).
  - **(ii-d) embed-and-compile-per-case fallback:** `run-ml` (Rust) reads the corpus source, generates a tiny
    `.cdz` driver with the program bytes as a literal `[...]` `List(Int64)`, compiles+runs it, reads the
    value. No boundary-arg needed (bytes are a compile-time literal). Heavier (compile per case) but always
    works. Likely the pragmatic first cut; (ii-c) optimizes it later if `--arg` lists work.
- FIRST BUILD UNIT (mine, when trunk-stub-landed [done] + box quiet): the Cadenza **s-expr-prefix reader**
  (bytes → `Node` arena for the int/bool + let/if + prefix-ops subset), + the driver, + wire `run_run_ml` to
  invoke it. Smallest slice: `(input 42)` and `(if false 1 2)` flip decline→value.

⚠️ **VALUE-FORMAT CORRECTION (2026-07-17, read `run_program_rust`/`run_program_wasm`): `run-ml` must emit the
value as a BARE SCALAR, NOT `(: N Type)`.** The gate captures rcdzc's/wasm's `Ran::Value` in "cdz-run's
BARE-SCALAR rendering" — `42`, `true` (main.rs:1034-35), NOT the corpus's `(: 42 Int64)` output form. The
DIFFERENTIAL compares `ml_value == rcdzc_value` as those rendered strings. So my `run-ml` verdict must be
`value 42` / `value true` (bare scalar, matching cdz-run's Display), NOT `value (: 42 Int64)`. Emitting the
`(: … )` form would falsely DISAGREE with rcdzc on every case. (The corpus's own `(output (: N Type))` is
only what rcdzc is graded against; the ML↔rcdzc differential compares the two BARE renderings to each other.)
Confirm cdz-run's exact int/bool rendering when building (bare `42`/`true`; negative `-4`; big-int form).

## FORK RESOLVED → C + DIFFERENTIAL TESTING (operator, 2026-07-17, via concierge assign)

Operator (verbatim): "The compiler ml needs a way to be compiled and then invoked the EXACT SAME WAY the
rcdzc does. That way we can start locking on behavior and differential testing. If it doesn't support
something it simply declines." → the A/B/C fork is RESOLVED to **C**, and the goal is sharpened from "X/N
conformance" to **DIFFERENTIAL AGREEMENT WITH rcdzc**:
1. **SYMMETRIC INVOCATION.** The ML compiler is invoked the SAME way the gate invokes rcdzc — source in →
   verdict out. That's `cdz run-ml` (already stubbed, `cef183504`/pending), the peer of `cdz compile
   --target rust`. The ML compiler becomes a peer GATE TARGET.
2. **DIFFERENTIAL vs rcdzc.** The gate runs BOTH rcdzc and the ML compiler on each shared-corpus program and
   DIFFS their verdicts — "locking on behavior". The grading rule:
   - ML **declines** (unsupported construct) → **coverage-not-yet, NOT a failure** (the climbing X/N).
   - ML **agrees** with rcdzc (same value) on a supported construct → PASS (counts toward X).
   - ML **disagrees** with rcdzc where it claims support (ran to a DIFFERENT value, or ran where rcdzc
     declined / vice-versa) → **FAIL** (a real differential miscompile — the only red).
   So the gate never compares ML to the corpus's own `(output …)` directly; it compares ML **to rcdzc's
   verdict on the same program** (rcdzc is the executable oracle). Decline is free; disagreement is the bug.
3. v-fleet-tooling owns the `cadenza-ml` GateTarget + the differential diff + the reported (non-baseline)
   `cadenza-ml: X agree / D disagree / N total` line. I own `cdz run-ml` + its source→arena front-end.

This is a SHARPER version of the filed plan: not "does ML match the corpus expectation" but "does ML AGREE
WITH rcdzc where supported." Same X/N-climbs-as-features-land property; the delete-hand-encoded-builders step
stands (the differential gate replaces them entirely).

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
