# Type inference is Hindley-Milner (unification, principal types, let-generalization)

*2026-07-04*

**What happened.** The type system's inference discipline is pinned: **Hindley-Milner /
Algorithm W** — inference by **unification** over type variables, yielding **principal (most
general) types**, with **let-generalization**. Cadenza is an **ML + LISP + Rust hybrid**; the ML
half means inference must be principled and rigorous, not ad-hoc.

**Why it came up.** cdz-rustc hard-seeded every function parameter as `Int64`, so a Bool/Float
parameter declined ("argument kind mismatch"). The tempting quick fix — "infer a parameter's kind
from its first call site's argument" — is exactly the ad-hoc guessing HM exists to replace: it's
order-dependent, doesn't propagate through the body, and breaks on polymorphism. The operator's
directive: do it the OCaml way. A parameter used in a Bool position (`(if p …)`, `(= p true)`) must
be Bool because **unification** propagates that constraint to every occurrence of `p` — including
its type at every call site — not because we peeked at one argument.

**The discipline (now normative in type-system.md §Inference):**
- **Unification.** Assign each unknown a type variable; each use imposes an equality constraint;
  solve by unification. A parameter's type is the solution, derived from all its uses at once.
- **Principal types.** The inferred type is the most general one from which every valid type is an
  instance — inference commits to no more than uses require.
- **Propagation.** A determined type reaches every occurrence of the binding, so a parameter is typed
  consistently in its body AND at every call site (this is what the ad-hoc approach couldn't do).
- **Let-generalization.** A let-bound definition with free type variables is generalized (∀), so it
  can be used at different instantiations — this is the SAME mechanism as generics being type-valued
  parameters ([[2026-07-04-generics-are-type-valued-parameters]]); a monomorphic instantiation is an
  instance of the generalized scheme. A variable still constrained by an enclosing binding is NOT
  generalized (no escaping the scope where it's being solved).
- **No solution → rejection.** Contradictory constraints on a variable (a use imposing Int where
  another imposes Bool) is a compile-time type error with a machine-readable code, never compiled.
- **Annotations unify, never override.** `(: e T)` adds the constraint `type(e) = T`; if it fails to
  unify, reject (CDZ0203) — the annotation participates in inference, it doesn't replace it.

**Consequence for cdz-rustc.** The coarse "seed Int64, refine return kind to a fixpoint" scheme is a
stopgap that only works because every realized function was Int64→Int64. It must be replaced by a
real inference pass: type variables per parameter/return, a unification walk over each body (and
across call sites), producing each function's parameter and return kinds as the unification solution.
This subsumes both the return-kind fixpoint (already added) and parameter-kind inference (the Bool-
parameter cases). `Kind` (Int64/Bool/Float64/Unit/Never) is the current monomorphic ground-type
lattice; full HM adds type variables over it and, later, the structural/nominal type universe.

**The requirements it drove.** type-system.md §Inference rewritten: §"Inference Is Principal-Type
Inference By Unification" (4 reqs), §"A Let-Bound Definition Is Generalized" (2 reqs), §"An
Unannotated Program Is Accepted When It Has A Valid Typing", §"Annotations Constrain, Never
Contradict" (unify-not-override). Composes with [[2026-07-04-static-typing-is-mandatory-post-pivot]]
(the seed now enforces typing) and [[2026-07-04-generics-are-type-valued-parameters]] (generalization
= generics). Implementation in cdz-rustc is the next step; the coarse kind-fixpoint is the interim.
