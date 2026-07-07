# A recursive Bool function's return kind is inferred branch-order-dependently — the same kind race as Tier 00, now on Bool

*2026-07-07*

**What happened.** Probing the reader's name matcher surfaced a return-kind-inference asymmetry: a
**self-recursive function that returns `Bool` declines when its recursive self-call is the `then`
branch and a `Bool` literal is the `else`** — but the mirror shape (self-call in `else`, literal in
`then`) compiles. The failing shape is exactly the reader's "all bytes equal so far, else fail" loop:
`(def (go b i n) (if (< i n) (if (= (byte a i) (byte lit i)) (go b (+ i 1) n) false) true))` — used as
an `if` condition it declines *"if condition is not Bool"*; returned directly it declines *"if branches
differ in kind"*. The boundary is exact and, on probing, broader than first documented:

- self-call in `then`, `Bool` literal in `else` → **declines** (even when the `if` is the whole body,
  not nested: `(if (< i n) (go (+ i 1) n) true)` declines too).
- self-call in `else`, `Bool` literal in `then` → **compiles**.
- the same self-call-in-`then` shape returning **`Int`** (`(if (< i n) (go … acc) acc)`) → **compiles**.

So the trigger is precisely **self-call-in-`then` + Bool-literal-in-`else`**: the return kind fails to
settle as `Bool`. It is not about the condition position (returning the value declines too, with a
"branches differ in kind" message) and it is not general recursion (the Int accumulator settles fine)
— it is Bool-specific and branch-order-specific.

**Why.** This is the *same defect family* as the Tier-00 compile-time-inlining blowup
([[threaded-compound-accumulator-inference-blowup]] / the front-rung learning): an **order-dependent
kind race** where a self-call's kind is a placeholder until the function's own kind is known, and the
`if`-result inference unifies the placeholder-`then` against the concrete-`else` in an order that locks
the result as non-Bool. Tier 00 was the `Heap` instance (a threaded compound accumulator inferred as a
scalar); this is the `Bool` instance. The fix is the one already proven there: **a concrete-kind branch
must pin the `if`'s result kind regardless of branch order — a Bool-literal sibling settles the result
to Bool, and a self-call placeholder yields to any concrete sibling** (the "more-defined kind wins" /
back-propagation rule). That the same race recurs on a new kind is itself the lesson: kind inference's
order-independence is not a Heap-specific patch but a property every kind needs, so the fix belongs at
the general `if`/`match`-result unification, not per-kind. The reader makes it load-bearing — its head
resolver *is* a recursive Bool `name-eq` in exactly the failing shape — so it blocks self-hosting until
the general order-independence holds.

**The requirement it drove.** A conformance case in `09-functions.sexp` — *"a self-recursive
Bool-returning function whose recursive call is the then-branch"* (`(def (go i n) (if (< i n) (go (+ i
1) n) true))`, `(go 0 3) → true`) — pins that a recursive function's return kind settles to `Bool`
independent of whether the self-call is the `then` or `else` branch. It records the true oracle and
scores **todo** (the seed declines it today "if condition is not Bool" — a rule the seed *should*
cover, an inference bug, not an unrealized capability, so todo not skip). It is deliberately distinct
from the existing mutually-recursive `even`/`odd` Bool case, where each branch is a Bool literal or the
*other* function's call; here the branch is the function's *own* call, which is the order-dependent
case. This is the current gate on the reader's name matcher, hence on self-hosting, and it is recorded
as SPEC-BACKLOG item 14 framed as what it is: not a new feature but a **generalization of the Tier-00
kind-race fix to every result kind** — a Bool-literal (or any concrete) branch must pin an `if`/`match`
result kind regardless of branch order.
