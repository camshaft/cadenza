# ML-compiler: nested parens `((5))` RUNTIME-HANGS (the compiled wasm infinite-loops; type-check is fine)

**Reporter:** v-compiler-ml · **Date:** 2026-07-17 · **Severity:** codegen (runtime non-termination — a HANG)
**Class:** parse-db mutual-recursion SCC codegen (sibling of the compile-hang + invalid-wasm SCC faces).
**Status:** QUARANTINED out of the gated corpus (was a FLEET BLOCKER — hung `cdz test compiler-ml` every batch). Root-cause open; routed to v-inference (SCC/paren-hang class).

## Symptom

A doubly-parenthesised integer program hangs at RUN time:
`cdz run` of `run([TLParen, TLParen, TNum 5, TRParen, TRParen])` (i.e. the source `((5))`) never returns.
- `cdz check` of the same program COMPLETES (exit 0) — so it is NOT a compile-time/type hang.
- SINGLE parens are fine: `(5)`, `(1 + 2) * 3`, `2 * (3 + 4)` all run correctly (gated + passing).
- Only NESTED `(( … ))` hangs.

## Why it's a codegen miscompile, not a logic bug

`parse-db`'s paren handling is a mutual-recursion group `parse-any → parse-term → parse-factor`, and
`parse-factor`'s `TLParen` arm calls back into `parse-any(ts, i + 1, tree)` (paren nests the full grammar).
The token INDEX strictly advances on every step (`i + 1` past each `(`, and `parse-any` runs over the shrinking
suffix), so the recursion is WELL-FOUNDED — a correct compiler cannot produce an infinite loop from it. The
emitted wasm nonetheless loops forever on the `((5))` input, so the parse-db SCC is being MIS-COMPILED (the
runtime face of the parse-db mutual-recursion SCC codegen bugs — cf. the compile-HANG in
`mlrepro-parse-if-cond-via-parse-bool-mutrec-hangs-compiler.md` and the invalid-wasm face in the
now-RESOLVED `mlrepro-mutrec-scc-growth-emits-invalid-wasm`).

## Reproducer

- FULL: `implementation/compiler-ml/src/conformance-db.cdz`, the quarantined case
  `_c-cx-nested-redundant-parens` (`[TLParen, TLParen, TNum 5, TRParen, TRParen]` → expected `Runs(5)`), fed
  through `run` (parse-db + eval-db). Hangs at run.
- Minimizing to a SMALL standalone mutual-recursion (`any ↔ factor` over a `LP/RP/N` token list, index
  advancing) did NOT reproduce the hang — instead it hit the SIBLING invalid-wasm SCC face ("failed to parse
  WebAssembly module"). So the runtime-hang trigger is SCALE/LAYOUT-emergent in the full parse-db SCC
  (~5-member group + the tail helpers), not a minimal shape — the same emergent character as the other two
  SCC faces. The full parse-db + the quarantined case is the reproducer.

## Expected

`((5))` should run to `5` (nested parens collapse to the inner value), like every other parenthesised program.

## Route

parse-db mutual-recursion SCC codegen (v-inference's SCC/paren-hang class — coordinate). Not an ML-`.cdz`
logic fix (the logic is well-founded); the fix is in the seed's SCC codegen. When fixed, un-quarantine
`_c-cx-nested-redundant-parens` (re-add to `corpus()` + a `conformance-cx-nested-redundant-parens` @test,
bump size-floor back to 50) and confirm the FULL `cdz test implementation/compiler-ml` completes.
