# An immutable heap is acyclic, so reference counting is complete — no GC, minimal host

*2026-07-04*

**What happened.** The memory model gets a direction: **immutable, persistent data structures at the
surface, realized by reference counting with in-place reuse, with the allocator emitted into the
component.** The chain of reasoning is the load-bearing part, not the choice of RC:

1. **A strict, immutable language cannot construct a cyclic value heap.** To make a heap cycle you
   must *mutate* an already-allocated value to point at a value allocated later (or tie a lazy knot).
   Strict evaluation forbids the knot; immutability forbids the mutation. So the value heap is a DAG.
2. **Recursion is not a counterexample.** A recursive `def` compiles so its self-reference is a
   *static* reference — by name, to code — not a heap pointer into a reference-counted cell. Recursive
   *data* is unconstructible without mutation. So recursion does not create heap cycles.
3. **An acyclic heap is exactly the condition under which reference counting is complete.** The one
   thing RC famously cannot reclaim — cycles — is the one thing this language cannot create. So RC
   alone is both sound and complete; no tracing collector and no cycle collector is needed.

**Why.** Three commitments already in the tree point here, and the operator confirmed the surface is
**pure/immutable with mutation reintroduced only as an effect** ([[2026-07-04-effects-are-algebraic-capabilities-are-boundary-effects]]):
- `memory-and-resource-model.md` already requires **no tracing garbage collector**, that **cleanup be
  a deterministic function of the source** ("released after its last use in a way the executable
  semantics defines"), and that **aliasing be statically disciplined**. Those invariants are not
  neutral — they describe reference counting with a reuse analysis, not a GC.
- The goal is to **run on minimal-capability WebAssembly runtimes**: the host should provide *linear
  memory and nothing else* — no collector, no runtime memory manager. That forces memory management to
  be **baked into the emitted program**. A bump/free-list allocator is a few hundred instructions
  emitted into the component; RC drop calls are emitted at the deterministic last-use points.
- The closest existing systems are **Koka (Perceus)** and **Lean 4 (RC + FBIP)** — a *functional*
  surface with deterministic reclamation and no tracing GC — and **MLKit** (region/arena allocation)
  as the alternative to note. None were in the inspiration list (Rust/OCaml/LISP/F#); they are the
  nearest art to what the invariants already describe.

**Consequences.**
- **Perceus-style reuse makes immutability free when unshared.** When the compiler proves a reference
  is unique (its count is 1), a persistent "update" reuses the cell **in place** instead of
  allocating; when the value is shared, it copies the spine and shares the rest. In-place reuse is
  unobservable *because* values are immutable and equality is structural
  ([[2026-07-03-one-accessor-modules-are-records]]), so it never threatens determinism. This reuse
  analysis is affine reasoning done **in the compiler, invisibly** — which is why linearity is not
  needed in the surface language ([[2026-07-04-linearity-is-surgical-not-core]]).
- **Deterministic drop points satisfy the existing cleanup requirement directly.** The RC decrements
  are a function of the source, so "released after its last use in a way the executable semantics
  defines" becomes a concrete, checkable property rather than a deferred implementation detail.
- **The allocator is part of the byte-reproducible output.** Because the compiler emits it, allocation
  behavior is a function of the Cadenza compiler alone — consistent with the whole-component-is-emitted
  discipline ([[2026-07-03-the-compiler-emits-the-whole-component]]) — and allocation stays accountable
  against the resource measure (Constitution V), so a program cannot allocate unboundedly without
  consuming fuel.
- **The acyclic invariant must be preserved on purpose, not by accident.** Recursive closures bind
  through static references; recursive data stays unconstructible without an effect. If a future
  generation adds a mutation *primitive* outside the effect system, RC completeness is lost — which is
  another reason mutation is confined to a handled effect over a pure state-passing implementation
  ([[2026-07-04-effects-are-algebraic-capabilities-are-boundary-effects]]) rather than a heap mutation.

**Domain fit.** The target programs — smart-contract-shaped units that run once, produce a result, and
tear down, and that must run in a constrained sandbox — are exactly where "no GC, deterministic
cleanup, allocator in the component, minimal host" is a feature rather than a constraint. Region/arena
allocation (free nothing until instance teardown) is a legitimate optimization for the run-once shape
and is recorded as an alternative choice, but it breaks under memory bounds and long-lived state, so
RC-with-reuse is the default.

**The requirements it drives.** `spec/capabilities/memory-and-resource-model.md` is tightened from
"no GC + deterministic cleanup + disciplined aliasing, discipline deferred" to name the realizing
discipline as **reference counting over an acyclic (immutability-guaranteed) heap with in-place reuse,
allocator emitted into the component**: an acyclic-heap invariant requirement (the property RC
completeness rests on), a requirement that the runnable form carries its own allocation and
reclamation (minimal host), and a requirement that reuse is unobservable (meaning-preserving under
structural equality). A new decision **`options/memory-ownership-model/`** records the concrete
choice: `reference-counting-with-reuse` (Perceus-style) as the default, `region-arena` as the
run-once alternative. This is an area the spec previously under-determined; the requirement pass in
this session writes the RFC-2119 sentences.
