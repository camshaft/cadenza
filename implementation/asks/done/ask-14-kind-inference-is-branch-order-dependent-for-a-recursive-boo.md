## 14. 🟢 Kind inference is branch-order-dependent for a recursive Bool return — FIXED 2026-07-07

**Finding.** A self-recursive `Bool`-returning function declines when its `if` body has the self-call
in the `then` branch and a `Bool` literal in the `else` (`(if (< i n) (go (+ i 1) n) true)` → "if
condition is not Bool" as a cond, "branches differ in kind" when returned). The mirror (self-call in
`else`) compiles, and an `Int`-returning version compiles — so it is `Bool`-specific and
branch-order-specific: the self-call's placeholder kind and the `Bool`-literal sibling unify in an
order that locks the return kind non-Bool.

**Why it touches the seed (not a spec gap).** This is the **same order-dependent kind race as Tier 00**
(which was the `Heap` instance — a threaded compound accumulator inferred scalar), now on `Bool`. It is
a seed inference bug, not a language-surface question — the corpus already records the correct behavior.
The fix is the proven Tier-00 one, generalized: a concrete-kind branch (a `Bool` literal, or any
concrete sibling) must **pin the `if`/`match` result kind regardless of branch order**, and a self-call
placeholder yields to a concrete sibling. The lesson for the operator: kind-inference order-independence
is a property *every* result kind needs, so the fix belongs at the general result-unification, not as a
per-kind patch — worth stating once in the seed's inference rather than re-patched per kind.

**Why it matters.** The reader's head resolver is a recursive `Bool` `name-eq` ("all bytes equal so
far, else false") in exactly the failing shape, so this is a current gate on the reader → self-hosting.

**Status.** ⚪ Seed work (SEED-GAPS "Tier 2d" recursive-Bool note; mislabeled — a distinct item). Pinned
by `09-functions.sexp` *"a self-recursive Bool-returning function whose recursive call is the
then-branch"* (`(go 0 3) → true`). Learning:
`spec/learnings/2026-07-07-recursive-bool-return-kind-inference-is-branch-order-dependent.md`.

**Update (2026-07-07) — 🟢 FIXED.** The seed made `if`/recursive-return kind inference order-independent
for Bool (a concrete-kind branch pins the result kind regardless of order), the generalization this item
called for. The corpus case flipped **todo → PASS** with no oracle change. This unblocked the reader's
`name-eq` byte-comparator (was dead code in exactly this shape) → the reader's name matcher is now live.
Third confirmation the order-independence rule is one property, not per-kind (Heap=Tier 00, Bool=this).
Consolidated in `spec/learnings/2026-07-07-the-name-matcher-unblocks-and-the-surface-language-composes.md`.

---
