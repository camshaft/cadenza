# compiler.cdz corpus harness

Runs the Cadenza-authored compiler (`compiler.cdz`) against the whole conformance corpus
(`spec/semantics/*.sexp`) and compares its output, per case, against the native reference
compiler `cdz-rustc`.

## Usage

```
python3 implementation/compiler/harness/run_corpus.py            # whole corpus
python3 implementation/compiler/harness/run_corpus.py spec/semantics/01-literals.sexp   # one file
```

Requires the seed built (`cargo build --release` in `implementation/seed`) and
`CADENZA_RUNTIME` — the script sets it. Run from the repo root.

## What it reports (per corpus case C, input E, wrapped as `P = (module case (def (main) E))`)

The classification is **VALUE-FIRST** — byte equality is NOT enforced (that's deferred until the
compiler matures). Each realized case is one of:

- **agree** — `compiler.cdz` emitted a component **byte-identical** to native `cdz-rustc`. Strongest;
  the eventual differential-gate bar.
- **soft** — bytes differ BUT `compiler.cdz`'s `run()` value **equals the corpus oracle**. FINE and
  expected — we deliberately live in this middle ground. Main source: native emits overflow-checked
  arithmetic *helper functions* (and even a dead helper for a folded `+`) while `compiler.cdz`
  const-folds, so the modules differ byte-for-byte yet compute the same value.
- **trap-ok** — the oracle expects a runtime **trap** AND `compiler.cdz`'s built component both RAN and
  trapped with **real logic before the trap** (a verified SEMANTIC trap). Distinguished from `trap-dc`
  by disassembling the entry func (`is_bare_decline`): a genuine trap carries a computational op —
  `(/ 5 0)` → `i64.const 5; i64.const 0; i64.div_s` (the const-folder deliberately does NOT fold a
  trapping division), a byte-range guard carries its check — whereas a decline is a bare `unreachable`.
- **trap-dc** — the oracle expects a trap but `compiler.cdz` **DECLINED the construct** (entry func is a
  bare `unreachable` from an unsupported construct lowered to `KError`). It traps, so a value-only oracle
  would score it `trap-ok`, but the trap is NOT for the semantic reason the case tests — **coincidental
  agreement, NOT conformance.** Verified 2026-07-07: all four trap-expecting cases realized today
  (`Bytes.of` out-of-range/negative/runtime, missing field) are `trap-dc` — `compiler.cdz` doesn't
  support `record`/`Bytes.of`, so it never examines the byte value (a VALID `(Bytes.of (list 65 66))`,
  which must NOT trap, also traps). When the construct gains real support its range-check trap carries
  logic → it moves to `trap-ok`, and a WRONG check surfaces as `hard` (or fails the in-range companion).
  This bucket is the honest frontier for trap oracles — read it as `decline`, not conformance.
  See `spec/learnings/2026-07-07-a-decline-that-lands-on-a-trap-oracle-is-coincidental-agreement-not-a-semantic-trap.md`.
- **hard** — `compiler.cdz`'s component RAN but produced the **WRONG value** (≠ oracle), OR returned a
  **value where a trap is required**. The signal that matters — a true miscompile. (An invalid/
  unrunnable emission lands in **error**, also a bug.)
- **decline** — `compiler.cdz` emitted a component that **traps** (`unreachable`) where the oracle
  wants a value: an unsupported construct the reader lowered to `KError`. The honest frontier.
- **error** — `compiler.cdz`'s component fails to validate/run (malformed emission). A bug — the
  reader produced invalid bytes instead of a clean trap; the next decline-don't-miscompile target.
- **n/a** — native didn't realize the program, or the case has no scalar value oracle (compound/
  float/string/rejection); skipped.
- **skip** — the input couldn't be `quote`+`Ast.encode`d to get its AST bytes.

Run with `-v` (or a single file) to also list the `soft` cases.

## Progress (2026-07-07)

Latest full run: **22 agree, 6 soft, 4 trap-ok, 0 hard, 93 decline, 0 error, 124 n/a, 5 skip.**

🎯 **`hard` = 0 AND `error` = 0 — decline-don't-miscompile is fully achieved at the corpus level.**
Every component `compiler.cdz` emits is now one of: byte-identical to native (22), value-correct but
byte-different (6 soft), a correct trap on a trap-expecting case (4 trap-ok), or a clean trapping
decline (93). It never emits invalid bytes, never computes a wrong value, and never returns a value
where the program is required to trap.

How we got here:
- **`hard` 3→0** — `(= f1 f2)` / `(= -0.0 0.0)` / string inequality returned `true` (want false)
  because the reader's major-7 branch mis-read a CBOR **float** `0xFB` as a bool, collapsing distinct
  floats to one value. Fixed: `read-node` accepts ONLY the two bool encodings under major 7 (info
  20/21); float/null/other majors → unknown-marker → `KError` → trap. → moved to `decline`.
- **`error` 16→12→0** — invalid-component emissions eliminated in two steps:
  (1) a bare name-ref not in the param/let env (`unit`, nullary ctor, free var) used to read as
  `NLocal -1` → `local.get -1` (invalid); now `read-node`'s major-6 arm checks `ienv-pos ≥ 0` and
  declines an unbound name → trap. (2) a **helper-first** module (func 0 is a param'd helper, `main`
  is a later def) used to emit invalid bytes: `entry-guard` stubbed func 0 to nullary but kept the
  other funcs, so `main`'s call to the now-nullary func 0 was an arity mismatch. Now `entry-guard`
  COLLAPSES a non-nullary-entry module to a LONE nullary KError trap — a valid trapping `run`.
- **`decline` = 93** — the honest frontier: records, modules, metaprogramming, functions-as-values,
  runtime `Bytes.*`/`String.*`, and (until seed gap **3m** — the compile-cost ceiling — is fixed)
  helper-first / positional-entry modules that would need the `main`-named-entry reorder to actually
  compute their value.

## ⚠️ COVERAGE CAVEAT: const-folding masks the runtime `lower` arms

Validated 2026-07-07 by injecting a deliberate miscompile (`KAdd` lowers to `i64.sub`) into a copy of
`compiler.cdz` and running the harness. On the corpus's arithmetic cases (`(+ 2 3)` etc.) the bug did
**NOT** surface — `hard` stayed 0 — because `compiler.cdz` **const-folds** constant-operand arithmetic
BEFORE `lower` runs, so the buggy `lower` arm never executes; the case came back `soft` (value-correct,
because the fold is correct). The bug only produces a wrong value on **runtime** operands: a hand-built
`(module m (def (main) (add2 5)) (def (add2 x) (+ x 2)))` with the bug returned `run()` = 3 (want 7).

Consequence: `agree`/`soft` on a constant-operand arithmetic case validates the **const-folder**, NOT
the runtime `lower` path. The corpus is heavy on constant operands, so most arithmetic `lower` arms are
under-exercised by this harness. This is why the earlier float miscompile only surfaced via `(= f1 f2)`
(distinct-value equality) rather than a plain float literal. To exercise a runtime `lower` arm, a case
must feed the operator a **parameter/call result** (non-constant) — the harness sees few such corpus
cases. A future improvement: synthesize runtime variants (wrap each scalar arithmetic case's operands
behind a nullary-`main`-calls-a-helper shape) so the runtime arms get covered. Noted, not yet built —
and it needs the `main`-named-entry reorder (gap 3m) to run helper-first shapes anyway.

## STATUS 2026-07-07: value-harness LIVE again; `compiler.cdz` entry is `(def (compile b) …)`

`compiler.cdz`'s shipped entry is `(def (compile b) (compile-bytes b))` — the real self-hosting seam,
built as a `cadenza:compiler/compile : func(list<u8>) -> list<u8>` component (gap 3l's build path).
That is the shape `component-check` drives — but `component-check` is **blocked on gap 3n** (the seed's
`compile`-RETURN retptr is misaligned when the INPUT length is not a multiple of 4; deterministic,
native cdz-rustc component passes). So the byte-level gate can't run over arbitrary corpus programs yet.

**This value-first script is LIVE again** — instead of patching a nullary `main` (gone), it now
**INJECTS** a temporary `(def (main) (compile-bytes (Bytes.of (list …))))` just before the module's
closing paren for each case. The seed picks `main` over `compile` for the entry, so `emit` frames it as
nullary `run` and we read the runtime `Value` (the component compiler.cdz built) — the same flow as
before, and it **sidesteps gap 3n entirely** (uses `emit`/`run()`, never the compile-return path). The
`compile` entry stays the shipped seam; the injection is harness-local, never written back.

**When gap 3n lands:** adopt `cadenza-seed component-check <compiler.cdz-as-compile-component>
spec/semantics` as the byte-level gate (grades SUCCESS cases; rejection cases still need the
diagnostics ABI — the `result<_, list<diagnostic>>` return + a way to construct diagnostics).
The classification logic here (agree/soft/trap/hard/decline) documents how to read those results.
