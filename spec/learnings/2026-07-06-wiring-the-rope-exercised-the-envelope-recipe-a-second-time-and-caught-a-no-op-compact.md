# Wiring the Bytes rope exercised the frozen-envelope recipe a second time — and caught a compact that did nothing

*2026-07-06*

**What happened.** The value-heap runtime had grown a *rope* representation for Bytes —
[a Bytes rope defers materialization behind the same observable bytes](./2026-07-05-a-bytes-rope-defers-materialization-behind-the-same-observable-bytes.md)
added `bytes-concat`/`bytes-slice`/`bytes-compact` at heap-interface indices 34–36, giving O(1) concat
and slice over shared byte leaves. Exposing them to the *language* meant the compiler had to import and
lower three new runtime functions, which is the **second** time the frozen component-emission envelope
was extended since it was baked (the first was the persistent vector,
[wiring the persistent vector re-derived the frozen envelope](./2026-07-05-wiring-the-persistent-vector-re-derived-the-frozen-envelope.md),
29 imports; this one takes it to 32). Three things fell out, and the interesting two are not the
envelope work.

1. **The re-derivation recipe is now genuinely mechanical.** The first envelope extension was an
   anxious, validated-at-every-step affair — author the extended reference in WAT, assemble with
   `wasm-tools`, split at the embedded core-module boundary, re-bake HEAD/TAIL/import constants, and
   *prove the splitter reproduces the existing constants byte-for-byte before trusting it on the new
   ones*. The second time, that same procedure ran start to finish without a single surprise: the
   splitter reproduced the 29-import HEAD (1440) and TAIL (400) exactly, the derived 32-import
   constants (HEAD 1612, TAIL 445, `RT_TAIL_PREFIX_LEN` 369→414) dropped in, and the re-derived
   envelope emitted a **valid** component that ran a runtime-concat program correctly *before any
   codegen logic changed* — isolating the envelope change from the behavior change. The "append-only,
   one-time re-derivation per batch" claim is no longer a hopeful assertion; it is a procedure that has
   now been executed twice with identical shape. What made it mechanical was keeping the *checks* from
   the first run (split-and-compare against the current constants; regenerate the N-version import
   content and diff it against the live constant), not the raw edits. The recipe's value is its
   self-verification, not its steps.

2. **Wiring the native op caught a compiler bug the const-fold path had hidden: `compact` did
   nothing.** Before this, runtime `Bytes.compact` was emitted as the *identity on the handle* — it
   returned its argument unchanged, with a comment reasoning that compact is "value-preserving, so at
   run time it's the identity." That reasoning is half-right and wholly wrong: compact is value-
   preserving, but its entire *purpose* is to change the representation — to re-base a slice's bytes
   into independent storage so a large pinned parent can be reclaimed
   (memory-and-resource-model.md #Retained Storage Is Accounted For What It Holds Live). An identity
   compact preserves the value and *keeps the parent pinned*, which is the exact resource leak compact
   exists to fix. The const-fold path masked this completely: every corpus compact case built its
   operand from literals, so it folded to a baked constant and the runtime `compact` code never ran.
   The op was "correct" on every test while being a no-op that defeated its own reason to exist. The
   lesson is the sharp one: **a value-preserving operation whose only effect is on resources is exactly
   the operation a value-only oracle cannot test** — the corpus checks the bytes, and the bytes are
   identical whether or not compact did its job. Wiring it to the native `bytes-compact` (which
   flattens the rope node to a fresh leaf and releases the children) is what makes it real, and the
   only way to have *known* it was a no-op was to route it to a runtime that observably re-bases. This
   is a general hazard for the whole resource-model surface (reset/reuse, dup/drop, compaction): their
   correctness is invisible to a structural-equality oracle, so they must be validated by construction
   (route to a runtime that actually reclaims) rather than by output comparison.

3. **A fallible runtime access needed its result *shape*, not just its result *kind*.** Runtime
   `Bytes.slice` is fallible — it returns `Option<Bytes>` (Some in bounds, None past-end or negative),
   the input-side companion of the O(1) concat. The inference already reported its kind (Heap, an
   Option sum) and constrained the sliced buffer to Heap. But the *renderer* is type-directed: to print
   the boundary result it walks a static `Shape`, and `shape_of` had no arm mapping `Bytes.slice` (or
   its cousin `Bytes.at`) to `Option<payload>` — so a program returning a runtime slice declined
   "cannot infer runtime compound result shape" even though the Sum-with-Bytes-payload renderer already
   existed. The fix was a small `option_shape(payload)` helper that builds the Option's `Shape::Sum` in
   the *same discriminant order* `variant_disc("Some"/"None")` reads at emit time — so the constructor
   and the renderer agree on which arm is which by drawing from one source (`sum_variants["Option"]`).
   The general point: for a runtime compound result, the seed needs the value's **shape** (to render
   it), which is strictly more than its **kind** (to type it); a fallible access that yields an Option
   must map to the Option shape with the *right payload shape* (`Bytes` for slice, `Int` for at), or a
   renderable value declines at the boundary.

The rope APIs are now first-class runtime Bytes operations: `Bytes.concat` is one native call (the
O(n²)→O(n) unlock a self-hosting compiler needs to assemble a module from encoded sections),
`Bytes.slice` is a fallible native slice rendered through its Option, and `Bytes.compact` is a real
re-base. Four corpus cases pin them on genuine runtime values (the pre-existing 28 all const-fold).

**Why.** The through-line from the vector to the rope is that the *runtime-side* bets keep paying out —
a tag-free heap where a new collection is a new arrangement of one node, and a fixed envelope extended
by appending — but the *compiler-side* work each append surfaces is a different lesson each time. The
vector surfaced a fixpoint hole in `if`-kind inference (a recursive builder under-resolved). The rope
surfaced two: an operation whose correctness the oracle structurally cannot see (compact), and the
gap between a value's kind and the shape needed to render it (fallible slice). Both are the same root
as the fixpoint hole — the seed's static reasoning (kinds, shapes, resource effects) is load-bearing
for more than the emitted program's *values*, and the corpus, being a value oracle, is blind to the
parts that don't move the bytes. The defenses are (a) route resource-only operations to a runtime that
actually reclaims, so their effect is exercised even when unobservable, and (b) treat "shape" as a
first-class obligation for any runtime compound result, distinct from its kind.

**The requirement it drove.** None new — this is engineering technique realizing existing requirements.
It is how `spec/capabilities/collections-and-text.md`'s byte-sequence operations extend to a rope
representation, how `spec/capabilities/memory-and-resource-model.md` #Retained Storage Is Accounted For
What It Holds Live's compaction escape hatch becomes real at the language level (not a no-op), and how
`spec/contracts/component-abi.md`'s frozen tag-free seam accommodates a third runtime-interface append
by the append-only re-derivation, now carried out twice. Recorded so the next append (a CHAMP map, an
RRB relaxation) inherits a recipe that is *demonstrably* mechanical, so the "value-preserving op the
oracle can't see" hazard is checked deliberately the next time a reset/reuse/compaction op is wired,
and so the "runtime compound result needs a shape, not just a kind" rule is not rediscovered the next
time a fallible access flows to the boundary. Composes with
[a Bytes rope defers materialization behind the same observable bytes](./2026-07-05-a-bytes-rope-defers-materialization-behind-the-same-observable-bytes.md)
(the runtime side) and
[wiring the persistent vector re-derived the frozen envelope](./2026-07-05-wiring-the-persistent-vector-re-derived-the-frozen-envelope.md)
(the first exercise of the recipe this one confirms is repeatable).
