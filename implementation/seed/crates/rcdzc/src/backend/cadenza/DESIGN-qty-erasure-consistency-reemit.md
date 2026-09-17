# DESIGN — consistent whole-subgraph Qty erasure in the cadenza re-emit

Owner: v-cadenza-backend. Status: PARTIALLY LANDED — operator DECISION seq-1044 ("don't decline or defer,
fix the compiler issue"). Scope: the `--target cadenza` re-emit of programs that STORE a quantity in a
collection (Map/List/Set/record) and then READ it back and consume it in an ERASING context (`Qty.value`
or a bare-numeric arith peel). Target cases: `spec/semantics/18-units-of-measure.sexp` 0261 + 0312.

## 0. RESOLUTION (supersedes the §4 pre-pass hypothesis)

The root cause turned out NOT to need a whole-subgraph pre-pass. Ground-truth tracing (see §2) showed the
map ELEMENT genuinely IS a `Qty` (the map's Core type stays `Map _ (Qty …)` — correctly), and the fix is to
consistently PEEL the Qty at the erasing CONSUMER — which the backend already does for `Core::Param`/
`Core::LocalRef` binders and for a `Core::SumPayload` binder, via `(. Qty value)` re-insertion. The bug was a
LOCAL inconsistency in the peel CONDITION:

- **0261 (LANDED):** the `Core::SumPayload` Qty.value-peel keyed on `type_of(id)` (the binder's OWN solved
  type, still `Qty`) instead of `eff_ty` (the VIEW-aware consumed type). When the arith operand-peel
  (`emit_operand`) re-emits a Qty binder operand with `view = Some(inner)`, `eff_ty` is the bare inner but
  `type_of(id)` is still `Qty`, so the peel was skipped → `q` emitted bare while the map element re-emits
  `(Qty.of …)` → `(+ Qty Int)` CDZ0501 / arms-differ CDZ0203. Fix: key the `SumPayload` peel on `eff_ty`,
  matching the `Param`/`LocalRef` binder-peels. One-line change; 0261 → PASS, ch18 cadenza 0 regressions.
- **0312 (REMAINING):** the SAME peel gap but for an OPAQUE Qty producer — `d = (Option.expect (Map.lookup …))`,
  a `Core::Call` result typed `Qty`, used as an operand of a bare-numeric arith. Unlike a binder, a Call
  result has NO `(. Qty value)` re-insertion: `emit_operand`'s `emit_expr_viewed(n, view=Some(inner))` sets
  `eff_ty = inner`, which SKIPS the `Ty::Qty` emit arm, so the Call arm emits the call typed `Qty` (ignoring
  the view) → `(+ Qty Int)`. Fix (next slice): in `emit_operand`, for a Qty operand that is an opaque
  producer (a `Core::Call`/member result — NOT a self-peeling const/arith/binder), emit it at its natural
  `Qty` type and wrap `(. Qty value)` so it peels to the inner. (A const/arith/binder keeps the existing
  `view = Some(inner)` peel — a const emitted at natural `Qty` DECLINES via `qty_disposition`, so the
  Qty.value-wrap must be scoped to opaque producers only.)

The §1–§6 material below is the original (now-superseded) analysis; read §0 for the actual resolution.

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
