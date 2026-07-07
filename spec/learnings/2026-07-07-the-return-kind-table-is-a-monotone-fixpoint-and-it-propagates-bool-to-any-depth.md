# The return-kind table is a monotone fixpoint, and it propagates a Bool result to any call depth — the capability gap 3k unblocked, now landed

*2026-07-07*

**What happened.** The compiler needs each function's **result kind** (i32 for Bool, i64 for Int) to type its
wasm signatures and its `call` sites: a function that returns another function's Bool result must be framed
`result i32`, not `result i64`, or the value it forwards mismatches its declared type. A *single-pass*
computation handles one level — a helper whose body is directly Bool-shaped (`(< a b)`, `(= n 0)`) is captured,
and a caller one step away reads it. But a **transitive** chain — `a` returns `b`'s result, `b` returns `c`'s,
and only `c` has a Bool body — is a **fixpoint over the call graph**: pass 1 learns `c` is Bool, pass 2
propagates to `b`, pass 3 to `a` and `main`. A single pass leaves the middle functions unresolved, and
defaulting an unresolved result to the integer type mis-frames them.

The spike landed exactly this: `build-ktab` / `ktab-iterate` compute the per-function return-kind table as a
**monotone fixpoint** — iterate `kind-of` of each body under the current table until it converges — and
`kind-of`'s `KCall` arm reads the table. Probing `compiler.cdz` directly (byte-comparing its output against the
native seed through the reader path) confirmed it works to arbitrary depth:

| program | native | compiler.cdz | byte-identical |
|---|---|---|---|
| depth-1: `main → lt → (< a b)` | 108 B | 108 B | ✅ |
| depth-2: `main → isLt → lt → (< a b)` | 124 B | 124 B | ✅ |
| depth-3: `main → a → b → c → (< x y)` | 140 B | 140 B | ✅ |
| depth-2 Int chain: `main → a → b → (+ x y)` | 164 B | 124 B | soft (value 8; native emits overflow helpers, mine folds) |

All the Bool chains frame **every** function `result i32`, byte-for-byte as the seed does; the fixpoint
converges at each depth rather than bottoming out after one propagation step.

**Why.** This is the capability that a prior cycle's blowup was blocking. The return-kind table *wanted* to be a
fixpoint from the start, but the seed's compile-time evaluator OOM'd on the fixpoint shape — a recursive
`iterate` that re-derives a value each round — so the compiler shipped a **single-pass** table as a stopgap
(captured a directly-Bool-bodied helper, but not a transitive chain). That blowup was
[[a-fixpoint-loops-blowup-is-fresh-re-seed-plus-list-result-not-the-loop]] (the fresh-re-seed-plus-list-result
conjunction) and its seed-side twin gap 3k; with 3k fixed on the seed side, the true fixpoint became
*expressible in the compiler's own source*, and `ktab-iterate` is the first real consumer of it. So the arc is:
a seed inference blowup → a stopgap single-pass approximation in the compiler → the seed blowup fixed → the
compiler's approximation replaced by the real fixpoint it always wanted. The lesson worth keeping is that **the
compiler's stopgaps are load-bearing markers of seed gaps**: the single-pass table was not a design choice, it
was the shape of a seed limitation showing through the compiler, and it resolved the moment the seed did — the
same way the entry-reorder is still a stopgap (positional entry) waiting on gap 3m
([[the-self-hosted-reader-compiles-a-multi-def-call-but-picks-the-entry-by-position]]). Reading the compiler's
"NOT YET / single-pass / reverted" comments is reading a live map of the seed's frontier.

**The requirement it drove.** A corpus case in `09-functions.sexp`: *"a boolean result propagates through a
three-deep chain of forwarding functions"* (`main → a → b → c → (= n 0)` → true, AGREE, byte-identical at
131 B). The existing *two*-function case pins one propagation step, which a single-pass table also passes; the
**three-deep** case is the one that distinguishes a fixpoint from a single pass — the middle functions `a`/`b`
are Bool only transitively, so a compiler that resolves result types in one pass (or defaults an unresolved
result to Int) mis-frames them, and only iteration to convergence types the whole chain. This is the
value-level requirement (`a(0) = true`) the compiler-internal fixpoint made visible; the corpus pins the
language fact (a transitive Bool chain is Bool at every level, to any depth), and the seed and compiler agreeing
byte-for-byte on it is the differential evidence the fixpoint is correct, not just value-lucky. General lesson,
a companion to the shifts-decline learning ([[a-no-scratch-local-lir-must-decline-ops-that-need-guard-locals]]):
where that cycle showed a stopgap DECLINE (the honest frontier of a fold-only backend), this shows a stopgap
APPROXIMATION (single-pass where a fixpoint was wanted) — both are the compiler's source honestly recording what
the seed could not yet support, and both resolve by the seed growing the capability, not by the compiler working
around it.
