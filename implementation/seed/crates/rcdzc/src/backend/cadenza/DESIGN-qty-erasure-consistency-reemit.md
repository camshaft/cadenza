# DESIGN — consistent whole-subgraph Qty erasure in the cadenza re-emit

Owner: v-cadenza-backend. Status: DESIGN (slice 1) — operator DECISION seq-1044 ("don't decline or defer,
fix the compiler issue"). Scope: the `--target cadenza` re-emit of programs that STORE a quantity in a
collection (Map/List/Set/record) and then READ it back and consume it in an ERASING context (`Qty.value`
or a bare-numeric arith peel). Target cases: `spec/semantics/18-units-of-measure.sexp` 0261 + 0312.

## 1. The symptom

Both cases PASS on wasm/rust but MISCOMPILE on the cadenza round-trip (HOP1 re-emit succeeds, HOP2
recompile of the re-emitted program fails):

- **0261** "a quantity read from a map COMBINES with a fresh same-dimension quantity":
  `(Qty.value (match (Map.lookup m 1) ((Some q) (+ q (Qty.of 5 meter))) ((None u) (Qty.of 0 meter))))`
  re-emits as `(match ((. Map lookup) #map((= 1 ((. Qty of) n meter))) 1) ((Some _m) (+ _m 5)) ((None) 0))`.
  HOP2: `CDZ0501 adding a quantity and a plain number` + `CDZ0203 match arms differ: (Qty Int64 meter) vs Int64`.
- **0312** "unit arithmetic on a map-extracted quantity re-enters the map typed": same shape via `Map.insert`,
  producing a double-Qty `(Qty (Qty Int64 meter) meter)` → `CDZ0201`.

## 2. Root cause — LOCAL disposition, GLOBALLY inconsistent

The re-emit decides per-node whether a `Ty::Qty` value is a SURFACE quantity (reconstruct `(Qty.of mag unit)`)
or an ERASED magnitude (emit bare), using the `expected` slot type (`mod.rs` ~1644/1665/1727) and
`qty_disposition` (`mod.rs` 6806). These two signals DISAGREE across a collection:

- The **map ELEMENT** (producer) is emitted with `expected = Some(Ty::Qty …)` — the map's DECLARED element
  type — so it WRAPS `(Qty.of n meter)`. The re-emitted map is thus typed `Map _ (Qty …)`.
- The **lookup BINDER** (consumer) has already been ERASED by the optimizer: `Qty.value` folded, so its
  Core solved type is the bare inner (`Int64`), and the peeling arith (`Core::Arith`, dimension adds no
  runtime op — `lower/arith_fold.rs`) emits it bare.

The `expected` type reflects the SOURCE element type; the consumer reflects the OPTIMIZED (erased) type. A
collection cannot be half-wrapped, so producer-wraps + consumer-reads-bare is an unrepresentable split.

## 3. The correct model

At runtime a quantity IS its bare magnitude — `(Qty.of 5.0 meter)` and bare `5.0` are byte-identical
(units-of-measure.md §Dimensions Are Checked Then Erased). So the optimized Core computes over bare
magnitudes everywhere, and collections STORE bare magnitudes. The `(Qty.of mag unit)` surface is needed in
the re-emitted program ONLY where its STATIC TYPE must be `Qty` to type-check as the original did — i.e. at a
genuine HOST-BOUNDARY quantity escape (a def result / exported value typed `Qty`, or a collection that
ESCAPES as a Qty-typed collection), never for a value consumed internally by `Qty.value`.

Therefore a Qty node is **Surface** (reconstruct) iff its value flows to a host boundary AS a quantity
WITHOUT passing through an erasing op; otherwise **Erased** (bare). Crucially this is a property of the
value's WHOLE subgraph, so the producer and every consumer of one value share ONE disposition.

## 4. Option B — the disposition pre-pass (over the optimized Core)

Before emit, compute a map `StructId -> QtyDisp { Surface | Erased }` for every `Ty::Qty` node/binder in the
def, then thread it into emit (replacing the local `expected==Qty` wrap decision):

1. **Value-flow graph.** Union-find (or SCC) linking Qty nodes that MUST share a disposition: a binder and its
   references; a collection's element type and the values inserted into it AND the binders bound from its
   lookups/matches; `if`/`match` arms with their join; `let`/binder init with the binder.
2. **Seeds.**
   - `Surface`: a def result typed `Qty` that crosses to the host (`def_result_ty` = `Qty` on an exported
     def); a value stored in a collection/record that itself escapes as a Qty-typed collection.
   - `Erased`: the operand of a `Qty.value`; a Qty operand of a bare-numeric-result arith (the existing
     peel, `mod.rs` 2062).
3. **Propagate + resolve.** A linked group is `Erased` unless it flows UNPEELED to a `Surface` boundary. A
   group that is BOTH consumed-bare AND declared as a collection element defaults to `Erased` (the storage is
   bare; there is no boundary escape) — this is exactly 0261/0312. A genuine escaping-Qty-collection stays
   `Surface` (§Slice 3).
4. **Emit.** At a Qty node, consult the pre-pass disposition instead of `expected`:
   `Surface` → reconstruct `(Qty.of mag unit)`; `Erased` → emit bare magnitude (peel binders via the existing
   `(. Qty value)` re-insert). The collection element then emits BARE for 0261/0312 → the map is `Map _ Int64`,
   binders are `Int64`, arms are `Int64` → HOP2 type-checks and computes the identical bare result.

## 5. Slice plan

- **Slice 1 (this doc).** Design + grounding. No behavior change.
- **Slice 2.** Implement `QtyDisp` pre-pass (graph + seeds + propagation) and thread it into the `Ty::Qty`
  emit arms + `qty_disposition`, gated so a value with NO pre-pass entry keeps today's behavior (no regression).
  Target: 0261/0312 become TRUE cadenza PASSES.
- **Slice 3.** Verify/close the genuine ESCAPING-Qty-collection case (a def returning `Map K (Qty V u)` must
  stay `Surface` — elements wrapped). Add a corpus case pinning it if absent.
- **Slice 4.** Broad Qty re-emit regression sweep (all of ch18 + any other Qty-collection cases across the
  corpus) as the send bar, since this touches the shared Qty re-emit path.

## 6. Risks

- Touches the shared `Ty::Qty` emit path → broad regression surface; Slice 4's sweep is mandatory.
- The optimizer's erasure means a node's Core type may already be bare where its binder's declared type is
  Qty; the pre-pass must key on the value-flow linkage, not any single node's type in isolation.
- Scaled/prefix units (non-1/1 scale) are an orthogonal existing limitation (a scaled runtime magnitude still
  declines pending a scale-multiply slice); this design does not change that and must not regress it.
