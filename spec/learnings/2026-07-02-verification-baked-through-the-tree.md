# Verification was baked through the tree

*2026-07-02*

**What happened.** Earlier Cadenza's design treated heavy verification machinery — contracts with
preconditions and postconditions, refinement types, an effect system, linear ownership, and SMT-based
proving — as always-on core language. The result coupled what could have been a small, learnable
language to a large verification apparatus: a program could not exist in a simple form because the
simple form and the fully-verified form were the same thing.

**Why.** There was no layering discipline. The "simple core" and the "prover" were designed as one
artifact, so the cost of the prover was paid by every program, and there was no way to grow a program
from a sketch to a proven artifact incrementally — it was proven-shaped from the first line or not
expressible at all.

**The requirement it drove.** [Core Principle VIII](../../constitution.md) "Verification Is
Progressive And Meaning-Preserving" and [verification-layers.md](../capabilities/verification-layers.md):
a program compiles when only the core guarantees hold, each layer is optional and meaning-preserving,
and — the load-bearing subtlety — whether an obligation is discharged statically does not change the
emitted bytes, so a nondeterministic solver never enters the reproducible byte path and a static
discharge is a reproducibly checkable certificate. Units of measure, the one verification-flavored
piece of the old identity worth keeping, survives explicitly as an *optional, compile-time-only*
layer ([units-of-measure.md](../capabilities/units-of-measure.md)), never baked into the numeric core.
