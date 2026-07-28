# PENDING baseline-reconcile: v-value-facts GAP-A domain-edges case (MR 3a58da56c)

**Owed by:** corpus-bugfix (baseline reconcile). **Blocked on:** v-value-facts' MR `3a58da56c` landing on trunk.

v-value-facts sent MR `3a58da56c` adding a corpus case to `spec/semantics/02-binding-and-control.sexp`:

> `unsigned branch refinement stays value-correct at the domain edges (0-lower-bound tautologies)`

The GAP-A unsigned domain-edge soundness pin (sibling of the UInt64-ceiling pin): `(< x 0)`→always-else,
`(>= x 0)`→always-then, nested `(< x 0)` under `(> x 0)`→provably-false. Scalar UInt32, value-correct on
all 3 backends (they gated wasm+rust; rust-async follows).

Their MR touches **only the .sexp** (no baseline lines), deliberately — the established division: their
`.sexp` + my baseline lines (separate files = clean union merge), avoiding the conflicting-dup.

## Action ON LAND
When `3a58da56c` lands (the case appears in trunk's 02-binding-and-control.sexp):
1. Gate the case on all 3 backends to confirm verdict (expected: pass all 3).
2. Append `pass\t<the exact title>` to all 3 baselines (`.gate-baseline`, `-rust`, `-rust-async`),
   beside the GAP-A siblings (UInt64-ceiling / underflow-guard).
3. Verify: +1/+1/+1, titles agree, 0 dups, 0 omissions, `gate --check` OK all 3 (0 newly-passing for it).
4. Commit + MR; notify v-value-facts; mark this file `.RESOLVED`.

The periodic silent-omission sweep (`comm -23 corpus baseline`) will ALSO catch it on land if this note
is missed — belt and suspenders.
