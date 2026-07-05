# Memory Ownership Model — Choice: reference-counting-perceus

> **The default choice for the `memory-ownership-model` decision** (see [README.md](./README.md) for
> the decision and the requirements a choice must satisfy). It pins the concrete reclamation discipline.
> Rationale: `spec/learnings/2026-07-04-immutable-heap-is-acyclic-so-reference-counting-is-complete.md`.

## Reference counting is complete because the heap is acyclic

Cadenza values are immutable and evaluation is strict, so a value can only reference values that
already existed when it was constructed — the heap is a DAG with no cycles. Reference counting, which
is unsound only in the presence of cycles, is therefore both **sound and complete**: dropping the last
reference to a value reclaims it immediately and no value is ever leaked. This removes the need for a
tracing garbage collector and, crucially, for a cycle collector — both of which would introduce
timing that a deterministic component must not have.

## Perceus-style in-place reuse

Reference-count instructions are inserted precisely (Perceus): when a value's refcount is known to be
1 at the point it is consumed, its storage is reused in place to build the result. This makes a
"persistent" update (e.g. functional record update) allocate nothing when the input is unshared, so
immutability does not cost a copy on the common linear-use path.

## The allocator is emitted into the component

The host provides only linear memory. The allocator and the reference-count runtime are emitted into
the component by the compiler, so reclamation is a property of the artifact, not of the host — and the
same component reclaims identically on every conforming runtime.

## Linearity is surgical, not core

Immutability plus reference counting already cover memory safety, so linear/affine types are **not**
mandatory core. Linearity is used surgically where the semantics need it — one-shot (affine)
continuations (effects-model), linear capability handles — and an optional usage layer may add more.
Graded/quantitative types with an erased `0` multiplicity are the course to aim at, not a bootstrap
requirement.
