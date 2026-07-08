# The recursive pattern check covers arity but not literal type or patterns under a constructor

*2026-07-07*

**What happened.** After the compiler agent fixed the nested-tuple-*arity* check (it made
`check_tuple_pattern_shape` recurse structurally), adversarial probing found two sibling gaps the
recursion still leaves open, both wrong-value miscompiles:

1. **Nested literal-type mismatch.** `(match (tuple 1 2) ((tuple true b) 9) (_ 0))` runs to `0`. The
   Bool literal `true` sits at a position whose scrutinee element is the Int64 `1` — a literal-pattern-
   type mismatch that is rejected CDZ0201 at the top level (`(match 5 (true 1) …)`), but nested inside a
   tuple pattern it silently not-matches and falls to the wildcard.
2. **Wrong-arity tuple pattern under a constructor pattern.** `(match (Some (tuple 1 2)) ((Some (tuple a
   b c)) 9) (_ 0))` runs to `0`. The three-element tuple pattern in `Some`'s payload faces a two-tuple —
   the same arity mismatch the fix just closed for tuple-in-tuple — but reached through a constructor's
   binder, which the recursion never enters.

**Why they are breaks.** core-semantics.md #Patterns Compose: a pattern "MUST admit any pattern in each
of its binder positions … matched recursively to any depth," and a wrong-arity/wrong-type arm is
ill-typed (02-binding-and-control.sexp §"a tuple pattern of the wrong arity is a type error", §"a
literal pattern's type must match the scrutinee's") — it MUST reject, "not silently fail." Both arms are
statically impossible to match, so returning the wildcard's value is a wrong result for a program that
must be rejected.

**Root cause — the recursion is structural over tuples only, and checks only arity.** In the seed
(`codegen.rs::check_tuple_pattern_shape`), the fix that closed nested-tuple-arity recurses element-wise:
`for (psub, ssub) in pattern[1..].zip(scrut[1..]) { check_tuple_pattern_shape(psub, ssub) }`. But (a) the
recursion only *enters* when the pattern node is a `(tuple …)` — so a tuple pattern nested under a
constructor pattern (`(Some (tuple …))`, whose root is `Some`, not `tuple`) is never descended into; and
(b) at each level it checks *arity and tuple-vs-sum kind* but never a nested *literal pattern's type*
against the scrutinee element. The top-level literal-type check (a separate `for arm` loop over
`literal_pattern_type(arm.first())`) likewise inspects only the outermost pattern. So both the
literal-type rule and the arity rule are enforced at depth only along the tuple-of-tuples spine, and only
for arity.

**The lesson (the sixth and seventh instances of one family).** This run's recurring defect is *a check
that covers only part of its obligation*, and fixing one facet does not fix the family: the arity
recursion was added, but the same recursion needed to also carry the literal-type check and to enter
through a constructor's binder, not only a tuple's element. Two independent things must generalize
together — WHAT is checked at each node (arity, kind, AND literal type) and WHERE the walk descends
(through every binder position: tuple elements AND constructor payloads AND list elements). A recursion
that carries one check down one spine looks like "the compositional rule is enforced" but only closes the
diagonal. The durable fix is one pattern-vs-scrutinee-shape walk that, at each node, dispatches on the
pattern's kind (tuple → arity + recurse elements; constructor → recurse payload; literal → type-check;
name/wildcard → ok) — so adding a new pattern kind or a new per-node check has exactly one place to live.

**Corpus cases added.** `spec/semantics/02-binding-and-control.sexp` §"a nested literal pattern of the
wrong type is a type error" (`(tuple true b)` vs `(tuple 1 2)`) and §"a wrong-arity tuple pattern nested
under a constructor pattern is a type error" (`(Some (tuple a b c))` vs `(Some (tuple 1 2))`), both MUST
reject CDZ0201. Native seed; the behavior gate catches both (expected reject CDZ0201, observed a running
component).
