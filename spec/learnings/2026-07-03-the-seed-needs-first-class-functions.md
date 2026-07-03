# The seed must realize first-class functions, because the first Cadenza artifact is a compiler

*2026-07-03*

**What happened.** The first cut of the seed reference interpreter realized only **top-level `def`s** —
named functions callable by name — and had no first-class function values: no lambdas, no closures, no
passing a function as an argument or returning one. That was enough to reproduce the then-existing
corpus. But once the staging collapsed so that the **first Cadenza artifact is the compiler**
([bootstrap is interpreter-first](./2026-07-02-interpreter-first-not-compiler-first.md), annotated),
the gap became load-bearing: a compiler is not expressible without first-class functions — its passes
map over trees with higher-order combinators, its environments carry closures, and its transformations
compose functions. The seed could not host a Cadenza-authored compiler as it stood.

**Why.** The realized-capability set of the seed is derived from *what a compiler needs*, not from a
minimal interpreter's needs (options/realized-capability-set/seed-ignition-set.md). An earlier note in
`options/bootstrap-interpreter-surface/` had even said higher-order functions were "not required at
this rung," reasoning from a meta-circular *interpreter* written with explicit recursion. That
reasoning does not survive making the compiler the first artifact: a compiler is the consumer, and it
needs functions as values. The lesson is that the seed's realized set must be scoped to the first
Cadenza artifact's needs, and when that artifact changed from "an interpreter" to "a compiler," the
realized set had to grow to match.

**The requirement it drove.** A new section in [core-semantics.md](../capabilities/core-semantics.md)
§"Functions":
- §"A Function Is A First-Class Value" — a function is a value that can be bound, passed, returned, and
  stored; a function value captures the bindings in scope where it is created (lexical closure).
- §"Applying A Function Binds Its Parameters To Its Arguments" — application evaluates the body in the
  captured environment extended with the parameters; applying to the wrong number of arguments traps of
  a defined kind (the `"arity mismatch"` trap, pinned in options/diagnostics-schema/).
- §"Recursion Is Accountable Against The Resource Measure" — self-application consumes the deterministic
  resource measure so unbounded recursion halts at a defined point.

Witnessed by `spec/semantics/09-functions.sexp` (core cases the seed runs: application, closure capture,
higher-order use, returning a function, arity-mismatch trap, bounded recursion, and unbounded recursion
halting as `(exhausted)`). The `fn` core symbol and the function-application form are pinned in
`options/code-shape/homoiconic-decoupled-display.md`, and functions are added to the seed's realized set
in `options/realized-capability-set/seed-ignition-set.md`.
