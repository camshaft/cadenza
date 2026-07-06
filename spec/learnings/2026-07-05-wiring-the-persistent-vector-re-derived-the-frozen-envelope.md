# Wiring the persistent vector re-derived the frozen envelope for the first time — and forced a fixpoint fix

*2026-07-05*

**What happened.** The value-heap runtime had grown a persistent vector (a 32-way radix trie: an
immutable, structurally-shared, growable sequence) at heap-interface indices 29–33 —
[a persistent collection fits the tagless heap with no new machinery](./2026-07-05-persistent-collections-fit-the-tagless-heap-with-no-new-machinery.md)
added it runtime-side with no new node field and no new reference-counting code. Exposing it to the
*language* meant the compiler had to import and lower five new runtime functions
(`vec-empty`/`len`/`get`/`push`/`update`), and that is the first time the frozen component-emission
envelope was actually **extended** since it was baked. Until now the envelope imported a fixed set
of runtime functions; the persistent vector is the first append. Three concrete things fell out:

1. **The "append-only, one-time re-derivation" claim was executed and held.** The fixed-envelope
   technique ([emitting a component with an import is a fixed envelope](./2026-07-05-emitting-a-component-with-an-import-is-a-fixed-envelope.md))
   bakes the component-model surround as constant HEAD/TAIL byte-arrays around a compiler-built core
   module, with every defined-function index shifted by a base equal to the import count. Adding five
   imports meant re-deriving those constants: the HEAD gained five instance-type entries + five
   `alias`/`canon lower` pairs (1200 → 1440 bytes), the TAIL gained five core-instance re-exports and
   its `run`/`cabi_realloc` core-func aliases shifted from 24/25 to 29/30 (344 → 400 bytes), the
   import section went 24 → 29 imports, and the import-count base `RT_N_IMPORTS` went 24 → 29. The
   re-derivation was done the prescribed dev-desk way — author the extended reference in WAT, assemble
   and validate with `wasm-tools`, split it at the embedded core-module section boundary, and re-bake
   the constants — with a split-and-compare check that reproduced the *existing* constants exactly
   before trusting it on the new ones. The design's promise ("the seam grows only by appending
   operations whose signatures name what a collection does") is now demonstrated, not just asserted.

2. **The re-derivation is genuinely a one-time cost per append, not per collection operation.** All
   five vector operations rode a single envelope bump; the marginal cost of a sixth would be another
   append, but the *technique* is now exercised and the split/compare harness makes the next one
   mechanical. The self-contained-component property (zero host-effect imports) and IGNITION's
   byte-identical self-reproduction both survived the bump — the envelope change is a fixed constant,
   not a live tool invocation, so a derivation's bytes stay a pure function of source.

3. **A recursive compound *builder* exposed a fixpoint hole in `if` inference.** The whole point of a
   persistent vector is incremental building: `(def (build v i n) (if (< i n) (build (Vec.push v i)
   (+ i 1) n) v))`. This declined with "if branches differ in kind." The cause: the seed infers a
   function's return kind by a fixpoint pass, and the `if` combined its branches with a then-biased
   `t.or(e)`. On the first pass the recursive-call branch reported the callee's *still-default* Int64
   return kind while the base branch `v` was already the heap value — so `t.or(e)` locked the `if`
   (and thus `build`'s return) to Int64, a fixpoint that never recovered even though `v` was plainly a
   vector. The fix: when the two branches disagree, `if` inference prefers `Kind::Heap` — the heap
   value is "more defined" than the unconstrained-parameter Int64 default, so a recursive builder's
   return kind converges to a heap value on the next pass. With it, `build` types correctly and its
   result is consumable (`Vec.len (build …)` → the element count).

The vector is now a first-class language value: `Vec.empty`/`push`/`update` build new versions
(persistent — the old version is untouched), `Vec.len`/`Vec.get` read, the type-directed renderer
prints `(vec e0 e1 …)`, and a runtime-length loop that accumulates a vector and then measures it
compiles and runs. Rendering a recursively-built vector *as the boundary result* still declines the
same way a recursively-built linked list does (its static shape is infinite) — off the critical path,
since a compiler consumes such a structure to bytes rather than returning it rendered.

**Why.** Two design bets paid out together. The runtime-side bet — a tagless heap where a new
collection is a new *arrangement* of one node — meant the vector cost the runtime nothing structural.
The compiler-side bet — a fixed envelope around a variable core module, extended by appending — meant
exposing it cost one mechanical re-derivation rather than a rewrite. The fixpoint hole is the
recurring shape of this whole effort: the seed's kind lattice is load-bearing for more than the
emitted program's correctness. Earlier it was
[a recursive consumer of a runtime heap value must be typed Heap, or the compiler diverges](./2026-07-05-a-recursive-consumer-of-a-runtime-value-must-be-typed-heap.md)
(a consumer under-constrained → the compiler hangs); here it is the dual, a recursive *builder*
under-resolved → the compiler declines a well-formed program. Both are the same root: an
under-determined kind at a heap boundary must resolve *toward* the heap, because the heap value is the
ground truth and the scalar default is only the absence of a constraint. Preferring Heap on
disagreement is the builder-side statement of that rule.

**The requirement it drove.** None new — this is engineering technique realizing existing
requirements. It is how `spec/capabilities/collections-and-text.md`'s sequence operations extend to a
persistent representation, and how `spec/contracts/component-abi.md` §"The Value-Heap Runtime Crosses
By A Well-Known Import" accommodates a growing runtime interface: by the append-only, one-time
re-derivation the fixed-envelope learning anticipated, now carried out and verified (IGNITION
byte-identity preserved, COMPONENT-CHECK native == component). Recorded so the next runtime-interface
append (a CHAMP map/set, an RRB tree) inherits the split-and-compare re-derivation recipe and the
`RT_N_IMPORTS`-shift discipline, and so the `if`-inference "prefer Heap on disagreement" rule is not
rediscovered the next time a recursive builder of a new heap type is authored. Composes with
[a persistent collection fits the tagless heap with no new machinery](./2026-07-05-persistent-collections-fit-the-tagless-heap-with-no-new-machinery.md)
(the runtime side) and [a recursive consumer of a runtime heap value must be typed Heap](./2026-07-05-a-recursive-consumer-of-a-runtime-value-must-be-typed-heap.md)
(the consumer-side dual of the fixpoint rule).
