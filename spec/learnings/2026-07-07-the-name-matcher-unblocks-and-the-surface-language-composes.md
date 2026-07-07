# The recursive-Bool fix unblocked the reader's name matcher — and the full surface language composes in one program

*2026-07-07*

**What happened.** The recursive-Bool return-kind race
([[2026-07-07-recursive-bool-return-kind-inference-is-branch-order-dependent]], SPEC-BACKLOG item 14)
is **fixed in the seed**. The corpus case that pinned it — *"a self-recursive Bool-returning function
whose recursive call is the then-branch"* — flipped **todo → PASS** with no change to the recorded
oracle, reject-don't-miscompile working exactly as designed. This directly unblocked the reader's name
matcher: `name-eq`, the byte-by-byte prelude-symbol comparator written in the natural
`(if (= a b) (recurse) false)` shape, previously declined "if condition is not Bool" and sat as dead
code ([[2026-07-07-the-reader-foundation-is-built-and-gated-on-one-inference-bug.md]]); it now compiles
and runs (comparing `b"++"` to `b"++"` byte-by-byte → 1). So the reader's last blocked primitive came
alive — the "build everything, park the blocked function as dead code" bet paid off exactly as
intended.

The more significant finding is an **integration milestone** the spike verified alongside the fix: the
full surface language composes in *one realistic program*, not just per-feature. A
`classify x = (if (and (> x 0) (< x 10)) (let ((y (* x x))) (- y 1)) 0)` called from `main` compiles
end-to-end to a valid component and runs — `classify 4 → 15`, `classify 20 → 0`. In that single
function, resolved from surface *names*, the pipeline composes: a **short-circuit `and`** of two runtime
comparisons (desugared at `resolve` to a nested `if (result i32)`), the **outer `if (result i64)`**
selecting Int64 branches, a **`let` binding a runtime value** `(* x x)` (emitting a real
`(local i64)` + `local.set`/`local.get`, not a compile-time alias), and the arithmetic — all threading
their values correctly through the emit. Every stage (resolve names → fold → lower → serialize → frame)
handled the composition, not merely each construct in isolation.

**Why.** Two durable points. First, the #14 fix is the third confirmation that **kind-inference
order-independence is one property, not a per-kind patch**: the same "a concrete branch pins the
result kind regardless of order" rule resolved the `Heap` race (Tier 00), and now the `Bool` race
(item 14), from the same root. That it recurred on `Bool` and was fixed by the same principle is the
evidence the rule belongs at the general result-unification, as item 14 argued. Second — and this is
the deeper lesson — **authoring the compiler surfaces gaps between individually-correct features that
no isolated case exercises** ([[2026-07-06-authoring-the-compiler-surfaces-gaps-a-corpus-grown-from-a-floor-misses]]),
and the *converse* is also true and worth pinning: when the compiler finally composes those features
in one program, that composition is itself a conformance obligation the floor-outward corpus lacks. A
`let` over a runtime value *inside* a conditional *guarded by a short-circuit `and`* is a shape the
corpus's isolated `let` cases (all top-level constant folds), isolated `if` cases, and isolated `and`
cases never jointly witness — yet it is exactly what real code writes and what a compiler must emit
correctly (a `let` here is a real wasm local, not the compile-time alias the folding cases exercise).
The integration program is the smallest witness that these compose, so it earns a corpus case in its
own right.

**The requirement it drove.** Two conformance cases in `02-binding-and-control.sexp` pin the
composition: *"a conjunction guards a let over a runtime value inside a conditional"*
(`classify 4 → 15` — the `and` true, the `let` binds `(* 4 4)` and computes 15) and its else companion
*"the guarded-let conditional takes its else-branch when the conjunction is false"* (`classify 20 → 0`
— the short-circuit `and` false, the `let`-bearing then-branch shielded). Together they pin that a
short-circuit `and`, a runtime `let` (emitting a real local), and an enclosing `if` **compose in one
function over a runtime parameter** and select by the runtime value in both directions — distinct from
the isolated cases because it is their composition that a real program (and the compiler itself) leans
on. Both PASS. The #14 fix needed no new case — its existing pin flipped green — and this cycle's work
consolidates the state: with #14 cleared, the reader's name matcher is live, and self-hosting's
remaining gates are the reader's top-level `read : Bytes → Node` wiring plus items 12 (symbol-table
`from-bytes`), 13 (list patterns), and 15 (the `let`-free `tuple.N` invalid-component decline). Also
noted for the spike: `compiler.cdz`'s header comment still calls `name-eq` dead code "until Tier 2d is
fixed" — now stale, since it is fixed; the reader is no longer gated on it.
