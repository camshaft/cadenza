# Decision — Memory Ownership Model

**The decision.** How a compiled component manages heap memory: the reclamation discipline, what the
host must provide, and where (if anywhere) linearity is required. The constitution requires
determinism and that the compiler emit no operation depending on uninitialized memory, but it does not
fix the reclamation strategy, which is what this decision pins.

**Requirements any choice must satisfy (from the spec — do not weaken):**
- The value heap is acyclic: immutability plus strict evaluation forbid heap cycles
  (memory-and-resource-model.md §The Value Heap Is Acyclic).
- Reclamation leaves no value uncollected and adds no nondeterminism (constitution III;
  memory-and-resource-model.md).
- The host provides only linear memory; the allocator is emitted into the component
  (memory-and-resource-model.md).

## Choices

- [`reference-counting-perceus`](./reference-counting-perceus.md) — because the immutable heap is
  acyclic, reference counting is sound **and** complete (no tracing GC, no cycle collector);
  Perceus-style in-place reuse makes persistence free when a value is unshared; the allocator is
  emitted into the component so the host provides only linear memory. Linearity is surgical (one-shot
  continuations, linear capability handles), not core. **The default.**

DEFAULT: reference-counting-perceus
