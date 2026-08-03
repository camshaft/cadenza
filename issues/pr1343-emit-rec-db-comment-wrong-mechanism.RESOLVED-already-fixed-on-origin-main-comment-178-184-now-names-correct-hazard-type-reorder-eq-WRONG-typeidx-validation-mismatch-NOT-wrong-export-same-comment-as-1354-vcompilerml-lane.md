# PR #1343 review comment — implementation/compiler-ml/src/emit-rec-db.cdz (v-compiler-ml)

Mirrored from https://github.com/camshaft/cadenza/pull/1343 (PR: "cand: v-compiler-ml — 614017827").

## Comment ties `recursive-types` ordering to the wrong failure mode (Copilot, emit-rec-db.cdz:182) — doc/correctness
> The comment here ties `recursive-types` (type-section ordering) to exporting the wrong function,
> but `export-section-idx(k)` is based on the *function* index and does not depend on type ordering.
> Reordering the type list would instead desynchronize `typeidx == funcidx` and likely lead to a
> validation/type mismatch, while exporting the wrong function would require reordering the function
> list (bodies/func section).

The comment's causal claim is wrong: reordering `recursive-types` (type section) would break the
`typeidx == funcidx` correspondence (validation/type mismatch), NOT cause the wrong function to be
exported (that's a function-list ordering concern). Reword so the comment names the actual hazard of
type-section reordering.
