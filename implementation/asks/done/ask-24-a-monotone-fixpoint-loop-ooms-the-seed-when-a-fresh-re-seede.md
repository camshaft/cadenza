## 24. 🟢 A monotone fixpoint loop OOMs the seed when a fresh-re-seeded list parameter is consumed as a list — RESOLVED (seed side) 2026-07-07

**🟢 RESOLVED 2026-07-07 (seed side) — verified by direct probe.** Both OOM reproducers below now COMPILE in
under a second (EXIT=0) where Run 47 saw multi-GB OOM at 40s: (a) fresh-`(list)`-re-seed + list result →
2,978-byte component; (b) the monotone-`recompute` fixpoint → EXIT=0. (Run verification needs the composed
runtime-heap host — these import `cadenza:runtime/heap` — so `wasmtime run` alone errors on the missing import;
the point is the compile no longer blows up.) **Consequence — the return-kind fixpoint LANDED in `compiler.cdz`:**
with 3k fixed, the true fixpoint became expressible in the compiler's own source, and `build-ktab`/`ktab-iterate`
(a monotone fixpoint over the FList) replaced the single-pass stopgap. Verified byte-identical to the seed for a
Bool chain at depth 1/2/3 (108/124/140 B, every func framed `result i32`). Pinned by `09-functions.sexp`
*"a boolean result propagates through a three-deep chain of forwarding functions"* (→ true, byte-identical
131 B — depth-3 is what distinguishes a fixpoint from a single pass). Learning:
`spec/learnings/2026-07-07-the-return-kind-table-is-a-monotone-fixpoint-and-it-propagates-bool-to-any-depth.md`.
(Original finding, with the four-control trigger analysis, kept below.)

**Finding.** The self-hosting return-kind machinery's next step is a monotone **fixpoint** (iterate a table
until it stops changing). The single-pass accumulator fix (item 18) shipped the SINGLE-PASS return-kind table,
but a fixpoint loop still blows the seed up to multi-GB RSS and is killed (`emit`, `ulimit -v 4G`, times out at
30–40s). **`compiler.cdz` needs this to iterate its return-kind table to a true fixpoint** (a depth-2 Bool chain
— a helper whose body is only a call to a Bool helper — needs the fixpoint the single-pass fix doesn't reach).

**Corrected trigger (probed 2026-07-07 — narrower than the SEED-GAPS doc's "fresh re-seed" description).** The
blowup is a **conjunction**, not a single condition. Four controls, run directly against the seed:

| # | shape | result |
|---|-------|--------|
| (a) | `(def (iterate ktab passes) (if (< passes 1) ktab (iterate (list) (- passes 1))))`, `(List.len (iterate (list 1 2 3) 2))` — fresh `(list)` re-seed, list result | **OOM** |
| (b) | `match`-driven `recompute` re-seeded `(list)` inside a fixpoint `iterate`, list result | **OOM** |
| (c) | thread the SAME list param unchanged through the fixpoint, list result | compiles (11,971 B) |
| (d) | fresh `(list)` re-seed each round, result consumed as **Int64** (`List.len` inside) | compiles (633 B) |
| (f) | thread the list and GROW it by `List.push` each round, list result | compiles (12,008 B) |

So the necessary conditions are BOTH: (i) the list parameter is re-seeded with a fresh `(list …)` literal each
round — a value NOT derived from the incoming parameter — AND (ii) the recursion's result is consumed as a list.
Threading the incoming list (c/f), even growing it by `List.push`, compiles; re-seeding fresh while consuming
the result as a scalar (d) compiles. The doc's one-variable trigger ("fresh re-seed") over-broadly condemns (d),
which compiles, and misses condition (ii) — **a fix must target both conditions, not the re-seed alone.**

**Likely mechanism (to confirm).** Same class as the fixed `eval_const` let-memoization blowup and the Tier-00
threaded-accumulator inference blowup — an inference/fold fixpoint that fails to reach a fixed KIND and
re-expands. When the parameter is re-seeded with a literal (not threaded), the incoming value gives no kind
constraint at that argument position, so each pass re-derives it; if the result is also a list, the return-kind
back-propagation (the very machinery item 18 added) must reconcile "fresh literal at the call site" against
"heap result at the use site" every iteration, re-triggering the inline/fold expansion instead of converging.
Threading (c/f) pins the parameter's kind once; a scalar result (d) removes the return-kind constraint.

**Acceptance signal.** Reproducers (a) and (b) `emit` to a valid component within seconds (not OOM); `(List.len
(iterate (list 1 2 3) 2))` = 0 (each round discards the incoming list for a fresh empty one → final `()` →
len 0). Then the return-kind table can iterate to a true fixpoint and the depth-2 Bool chain compiles.

**Pinned (passing side of the boundary — the OOMing program can't be a corpus case, it hangs the gate).**
`05-compound-types.sexp` *"a fixpoint loop that threads a growing list accumulator returns that list"*
(`(List.len (loop (list 1 2 3) 2))` = 5, AGREE) — proves threaded list accumulators in a fixpoint are
representable today and marks exactly where the frontier begins.
Learning: `spec/learnings/2026-07-07-a-fixpoint-loops-blowup-is-fresh-re-seed-plus-list-result-not-the-loop.md`.

---
