# A recursive cons-list→Bytes fold now infers its shape as the direct result — the serialize spine, no concat anchor needed

*2026-07-07*

**What happened.** The seed rebuilt and fixed Tier 3d: a recursive function that folds a cons-list of
byte fragments to `Bytes` — `(match xs ((Nil) empty) ((Cons (tuple h t)) (Bytes.concat h (rec t))))` —
now compiles when its call is the program's **direct result**, where it previously declined "cannot
infer runtime compound result shape" unless the fold was anchored by a literal `Bytes.concat` operand.
Verified: a runtime-built (length-driven) cons-list `[b"C" b"B" b"A"]` folded by `cat-all` as `main`'s
whole result → `b"CBA"`. This is the compiler's **serialize spine** — folding a code stream (a list of
encoded instruction/section fragments) into the output byte vector — and it now works in its natural
form, not only when a stray literal operand happens to pin the shape.

**Why.** The gap and its earlier workaround are both instructive about *shape inference through
recursion*. The result shape of `cat-all` is "Bytes," but the compiler infers it from the function
body, and the body's two arms are `(Bytes.of (list))` (clearly Bytes) and `(Bytes.concat h (cat-all
t))` — where the recursive call `(cat-all t)`'s shape is a placeholder until the function's own shape is
known. Before the fix, when that fold was `main`'s direct result, the placeholder-vs-Bytes unification
didn't settle to Bytes; the workaround was to wrap it in a `Bytes.concat` with a shape-anchoring operand
(even a literal `(Bytes.of …)`), which let inference conclude Bytes from the *other* operand. This is the
**same family as the recursive-Bool return-kind race** ([[2026-07-07-recursive-bool-return-kind-inference-is-branch-order-dependent]])
and the Tier-00 Heap kind race: a self-call's kind/shape is a placeholder during the function's own
inference, and the fix is to let a concrete sibling (the `Bytes.of (list)` base arm, or the concat's
literal operand) pin the result regardless of where the placeholder appears — order/position
independence in shape inference, now extended to the *compound-shape* axis as it was earlier to the
`Heap` and `Bool` kind axes. The recurring lesson holds: **inference of a recursive function's
result — kind or shape — must be independent of which branch/operand the self-call sits in; a concrete
sibling pins it.** The concat-anchor workaround was the same shape of contortion as the earlier
Bytes-hack (a spurious operand added to satisfy inference); removing the need for it is the honest fix.

**The requirement it drove.** A conformance case in `10-bytes.sexp` — *"a recursive fold of a cons-list
to bytes is the whole program result"* — pins the serialize spine: `cat-all` folds a runtime-built
`BL` cons-list of `Bytes` fragments by recursive `Bytes.concat`, as `main`'s direct result,
`cat-all (build 3) → b"CBA"`. It is deliberately the *list-fold* companion of the existing per-node
*tree-walk* emitter case (`Expr → Bytes`): there the recursion is over a sum's sub-nodes; here it is
over a cons-list's spine, and the fold is the whole result with no anchoring operand — the exact shape
3d gated. It **PASSES**. No new backlog item — this is a seed inference fix (Tier 3d) closing, the
compound-shape instance of the recurring "recursive result inference must be position-independent"
family; the standing frontier remains the compiler *emitting* its own richer constructs (`match` on
user sums) and scale (TCO). One note for the ledger: the still-declining "recursively-built linked list
renders its full runtime spine" todo is *not* this shape — that returns a recursive-sum *value* to be
rendered (an unbounded static render shape, a genuine decline), whereas this returns *Bytes* (a
determinate shape); the two look similar but the Bytes fold has an inferable result and the sum-value
render does not.
