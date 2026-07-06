# The compiler's own I/O type must be a first-class runtime value, and the frozen import set decides which

*2026-07-05*

**What happened.** The self-hosted compiler's seam is `compile : list<u8> -> result<list<u8>, …>`
([the compile seam is statically typed](./2026-07-03-the-compile-seam-is-statically-typed.md)) — it
**reads** input bytes and **builds** output bytes whose contents depend on run-time data. Yet in the
seed, `Bytes` existed only as a *compile-time* value: every corpus case built bytes from literal
integers (`(Bytes.of (list 1 2 3))`), so `eval_const` folded the whole value to a `CVal::Bytes` and
baked one constant string; a byte sequence carrying a genuine *runtime* byte could not be produced at
all. That is the compiler's own input and output type unavailable as a value the compiler could
manipulate — a direct wall in front of self-hosting.

Runtime `Bytes` was added: construction (`Bytes.of` with a runtime element), measurement
(`Bytes.len` → a scalar), concatenation (`Bytes.concat`, a copy loop into a fresh buffer), and the
recursive-builder idiom (`(rep n)` = concat a fragment `n` times → the shape a compiler emits a
component's bytes with). All build on the value-heap runtime's `bytes-alloc`/`bytes-set`/`bytes-get`/
`bytes-len` operations. The behavior gate rose to 428 passing with the pre-existing FAIL set
unchanged; IGNITION byte-identical; the wasm compiler component agreed with native on 433 programs.

Two facts shaped *which* type became a runtime value first, and *how* its edges behaved:

- **The frozen import set decided the order.** The emitted program is a fixed component envelope
  around a compiler-built core module
  ([emitting a component with an import is a fixed envelope](./2026-07-05-emitting-a-component-with-an-import-is-a-fixed-envelope.md)),
  and that envelope imports a *fixed, ordered* set of runtime operations. The `bytes-*` operations
  were already in it (they had been reserved when the runtime interface was frozen), so runtime Bytes
  needed **no** envelope re-derivation — it was emittable immediately. Runtime `String`'s operations
  (`str-new`/`str-get`) were **not** in the envelope, so runtime String would require re-deriving the
  frozen envelope and was deferred. The frozen import list, not the difficulty of the feature, set
  the sequence: *the cheap next step is whichever runtime value the envelope already imports.*

- **A range check must live on the language's side of a truncating primitive.** A Cadenza byte is
  `0..=255` on both ends, and constructing a byte outside that range MUST trap (a bounded operation
  with no defined out-of-range result). The runtime's `bytes-set` takes a machine word and truncates
  (`value as u8`), so if the bound were left to the runtime, `-1` would silently become `255` and
  `256` become `0` — a defined-outcome violation. The trap therefore had to be emitted by the
  compiler *before* the value reaches the truncating primitive, checking the full range on the
  Cadenza value. This is the general shape whenever a total-looking runtime primitive is narrower
  than the language value it backs: the language owns the check, the primitive does the store.

**Why.** Self-hosting is reached feature by feature, and the features that matter first are the ones
the compiler *is written in* — not an arbitrary breadth of the language, but the specific operations
the `bytes → bytes` core performs on itself. The compiler's substance is: consume an input byte
sequence, dispatch over an AST sum, and assemble an output byte sequence by recursive concatenation.
Runtime sum-match delivered the middle; runtime Bytes delivers the two ends. Prioritizing by
"what is the compiler's own I/O type" is a sharper heuristic than "what language feature is missing,"
because it points at the load-bearing few among many. And the frozen-import-set observation
generalizes the prioritization further: among the compiler's needed runtime values, do first the one
the emission envelope already admits, because it costs no contract change — a reproducibility-preserving
ordering that falls straight out of the fixed-envelope emission technique.

**The requirement it drove.** No new language-level requirement — `Bytes` and its `0..=255` range,
its fallible indexing, and the byte-sequence value form were already specified
(collections-and-text.md; [the seed realizes a byte-sequence form](./2026-07-03-seed-realizes-bytes-so-the-compiler-emits-components.md)).
This learning records a **realization milestone and two engineering invariants** on the road named by
[self-hosting is gated on generics; the rest is libraries and scale](./2026-07-05-self-hosting-is-gated-on-generics-the-rest-is-libraries-and-scale.md):
the compiler's output type is now a first-class runtime value (build/measure/concat/recursive-build),
while its *input* consumption (indexing a runtime buffer and matching the resulting `Option`) remains
blocked on runtime polymorphic-payload sum-match — the same unresolved increment
([the runtime is tag-free](./2026-07-05-the-runtime-is-tag-free-rendering-walks-a-static-shape.md)
governs the sum machinery this needs). The invariants: **(1)** among a compiler's needed runtime
values, realize first the one the frozen emission envelope already imports — it needs no contract
re-derivation; **(2)** when a runtime primitive truncates a value the language bounds more tightly, the
compiler emits the range check on the language value before the primitive, so a partial operation's
trap is not silently swallowed by the primitive's truncation (core-semantics.md §Partial Operations
Have A Defined Outcome, enforced on the emitting side).
