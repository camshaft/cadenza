# Decision — Effects Model

**The decision.** The concrete shape of Cadenza's effect system: how a host-delegated effect is called
at the boundary, how an intra-program effect is raised and handled, and how the effect row is
tracked in the type system. The constitution and `capabilities-and-effects.md` require that a program's
escaping effects equal its manifest and that reaching an undeclared capability is a compile-time error,
but they do not fix the operational mechanism, which is what this decision pins.

**Requirements any choice must satisfy (from the spec — do not weaken):**
- A program's escaping effect row equals its imported host functions; purity is the empty row
  (host-interface-binding.md §The Manifest Is A Projection Of The Escaping Effect Row).
- Reaching a host function the manifest does not enumerate is a compile-time rejection
  (capabilities-and-effects.md §Undeclared Capability Is A Compile-Time Error).
- A host call is a plain call to an imported function that returns its response; a run is a
  deterministic function of its input and the ordered responses to its host calls, and how the host
  resolves a call (inline, fiber-suspend, or re-derive) is host policy the language does not represent
  (capabilities-and-effects.md §A Host Call Returns A Response, §A Run Is A Deterministic Function Of
  Its Input And Responses).
- Determinism is never downgraded (constitution III; Governance Floors). (Bounded termination is no
  longer a language requirement — constitution Principle V, retired by Amendment 0.7.0.)

## Choices

- [`algebraic-one-shot`](./algebraic-one-shot.md) — a host-delegated effect is a plain imported-function
  call the host resolves (a run is deterministic in its input and ordered responses; resumption strategy
  is host policy); intra-program effects are algebraic operations discharged by lexically scoped handlers
  with one-shot (affine) continuations; the effect row is row-polymorphic and monomorphized to a closed
  set before the boundary. **The default.**

DEFAULT: algebraic-one-shot

## Operational lowering

[`algebraic-one-shot`](./algebraic-one-shot.md) fixes *what* effects mean; it does not fix how the
intra-program `handle` / perform / `resume` layer is transformed into WebAssembly. That operational
lowering is pinned in its companion:

- [`lowering-to-wasm`](./lowering-to-wasm.md) — the **classification-first** lowering: a compile-time
  pass sorts each handler arm into tail-resumptive / abortive / general-one-shot and emits a minimal
  stock-wasm shape for each. Because handlers resolve lexically at compile time over a monomorphized
  closed row, and every corpus arm (and the self-hosting compiler's own effects) is tail-resumptive, the
  shipping surface lowers with no continuation machinery — perform is a direct call to the statically
  known arm and a tail `resume e` is just `e`. The general-one-shot fallback reifies a continuation as a
  defunctionalized frame on the frozen value-heap prefix, so no arm of the design changes a frozen
  contract. It also fixes the composition invariant with the host boundary (a reified intra-program
  continuation must not span a host suspension point). This pins the operational mechanism the spec's
  behavioral requirements admit; it adds no requirement and weakens none.
