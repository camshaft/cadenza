# SMITH FINDING — backends DISAGREE on value: [value] wasm=(tuple (tuple 21.04 (tuple 21.04 (tuple 21.04 (tuple true "")))) (tuple (tuple true "") (tuple (tuple 21.04 (tuple 21.04 (tuple 21.04 (tuple true "")))) unit))) rust=(tuple (tuple 21.04 (tuple 21.04 (tuple 21.04 (tuple true "")))) (tuple (tuple true "") (tuple (tuple 21.04 (tuple 21.04 (tuple 21.04 (tuple true "")))) (tuple))))

_Filed by `cdz-smith` (the compiler fuzzer). This is an auto-generated finding: a
generated program made the compiler PANIC, HANG, or emit INVALID wasm — never valid
behavior, since the compiler reports every legitimate "no" as a diagnostic. Triage, fix,
then rename this file `.RESOLVED.md` (or `.REJECTED.md` with a rationale) so it is not
re-triaged._

- **Category:** differential
- **Compiler commit:** `8b6a415c1`
- **Hits:** 1
- **Signature:** `differential-value-wasm-tuple-tuple-tuple-tuple-tuple-true-tuple-tuple-true-tupl`

## Reproducer

`differential-value-wasm-tuple-tuple-tuple-tuple-tuple-true-tuple-tuple-true-tupl.smith.sexp` (also inline):

```scheme
(do (def (main) (let ((v0 (tuple 21.04 ))) (tuple v0 (tuple  (tuple v0 (tuple)))))) (export main))
```

Reproduce in-process:

```
cargo run -p cdz-smith -- verify differential-value-wasm-tuple-tuple-tuple-tuple-tuple-true-tuple-tuple-true-tupl.smith.sexp
```

## Backend value disagreement (miscompile)

Both emit backends produced a VALID artifact, but they DISAGREE on the program's
result — the wasm component (run via `cdz-run`) and the Rust backend (run via
`cdz run-rust`) computed different values, or one ran to a value where the other
trapped. The backends share the front-end and diverge below the emit seam, so this
is a lowering bug on one side. The crash/invalid-wasm oracles are blind to it.

- **Disagreement:** [value] wasm=(tuple (tuple 21.04 (tuple 21.04 (tuple 21.04 (tuple true "")))) (tuple (tuple true "") (tuple (tuple 21.04 (tuple 21.04 (tuple 21.04 (tuple true "")))) unit))) rust=(tuple (tuple 21.04 (tuple 21.04 (tuple 21.04 (tuple true "")))) (tuple (tuple true "") (tuple (tuple 21.04 (tuple 21.04 (tuple 21.04 (tuple true "")))) (tuple))))

## FUZZER TRIAGE (verified + shrunk, 2026-07-25, trunk 8b6a415c1)

VERIFIED reproducing via `cdz compile … -o w.wasm; cdz run w.wasm` vs `cdz run-rust` (NOT phantom).

**Minimal reproducer** (shrunk from the generated case):
```scheme
(do (def (main) (let ((v (tuple 21.04))) (tuple v v (tuple)))) (export main))
```
- wasm: `(tuple (tuple 21.04) (tuple 21.04) unit)`  ← trailing empty tuple renders as **`unit`**
- rust: `(tuple (tuple 21.04) (tuple 21.04) (tuple))` ← renders as **`(tuple)`**

**Isolation (which factors are necessary):**
- bare/top-level `(tuple)` → AGREES (both `(tuple)`).
- empty tuple nested/inline with Int64, or single float ref → AGREES.
- **Trigger = ALL of: a `let`-bound Float64-containing tuple `v`, referenced ≥2×, AND a sibling empty `(tuple)`.** Referencing `v` only once does NOT flip; using Int instead of Float does NOT flip.

**Direction (fuzzer read, for the fix agent — not a fix):** an empty-tuple element rendering as `unit` vs `(tuple)` is a compound-element render/representation divergence below the emit seam — same family as the earlier compound Float64-render bug (v-runtime `float_leaf` shortest-render path vs scalar `display_float`). The float+dual-ref condition suggests the wasm side collapses the zero-field tuple to the unit representation only when a shared/duplicated heap Float payload is in the compound (possibly a Perceus dup/share interaction picking the `unit` shell for the empty sibling). Rust backend keeps the nominal empty-tuple render. Which side is canonical (`unit` vs `(tuple)` for a zero-field tuple value) is a semantics call for the PM/fix agent.

🪤 to repro the WASM side of this s-expr: `cdz compile FILE.sexp -o out.wasm` then `cdz run out.wasm` — NOT `cdz run-ml` (declines s-expr) and NOT `cdz-smith verify` (crash-oracle only).
