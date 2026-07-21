# DESIGN — Rust backend Qty compound-value render scale-fold

**Status:** ✅ BASE LANDED (v-rust-backend, 2026-07-21, MR `0327858d4` → trunk `38509abe6`). Routed by
pr-sync + v-core-opt; v-quantity consulted. Option A (per-path `// cdz-qty-at` note) shipped for TUPLE /
RECORD / OPTION / RESULT Qty leaves.

⏭️ **FOLLOW-UP (scoped, not yet built): USER-DEFINED sum variant payloads** (v-quantity: `Circle(3km)` →
raw `3.0` on rust). EMIT: extend `collect_qty_scale_paths` with `db` to resolve variant payload `Ty`s
(`db.type_decl_by_occ(decl).variants[].payloads` → `eval::typeval_of` gives the `Ty::Qty` with its unit;
mirror `enums.rs::variant_payload_renders`), keying `<variant>?<idx>`. RENDER: a MONOMORPHIC user sum takes
the RECURSIVE-HELPER arm (`fn __render_<Sum>`, currently passes `logical_path=""`), NOT the generic-inline
arm (needs `!args.is_empty()`) — key the payload scale LOCALLY per variant arm (`<variant>?<idx>`), correct
for non-recursive; a self-recursive Qty-bearing sum is a rarer sub-case. Both render arms + the emit walk
must agree on the key. No corpus case yet (v-quantity holds the pin) — add a self-authored gate case + unit
test. Also a Qty in a LIST element remains raw (same follow-up class).

## The gap (rust-only red on 18-units-of-measure.sexp)
Cases: "a bare TUPLE of quantities renders each element scaled to its reference (mixed inner types)",
"an OPTION payload quantity renders scaled", "a NESTED tuple of quantities renders every element scaled
at depth" (all v-quantity's, added `9e74290ea` + siblings, WASM-baseline-only → rust/rust-async UNBASELINED
and genuinely FAIL). On `--target rust`:
- `(tuple (Qty.of 5.0 kilometer) (Qty.of 5 mile))` → EXPECTED `(tuple (Qty.of 5000.0 meter) (Qty.of
  201168/25 meter))` but RAN `(tuple (Qty.of 5.0 meter) (Qty.of 5/1 meter))` — RAW magnitudes, not scaled.

## Why (the architecture)
The rust backend erases `Ty::Qty { inner, unit }` to its INNER magnitude (unit is compile-time). It
DISPLAY-SCALES at the boundary via a per-EXPORT note pair emitted in `mod.rs` (`emit_signature`, ~782):
- `// cdz-unit[ident]: <value-form>` — the reference-unit value-form (`unit.at_reference()`).
- `// cdz-scale[ident]: num/den` — the unit's scale (only if non-1).
The gate harness (`cdz-rust-render::cdz_render_expr`) reads these and, for the TOP-LEVEL Qty result,
multiplies the magnitude (Float ×num/den IEEE, Int ×num/den trunc, Rational cross-mult exact — the Qty arm
~606-694). **These notes are keyed to the EXPORT ident and threaded ONLY to the top-level result** (lib.rs
comment ~465: "the corpus has no Qty [in compound]"). A Qty NESTED in a Tuple/Option gets `unit_scale=None`
→ renders RAW. And the OUTPUT type annotation carries units already at REFERENCE (`meter`), so the scale
(`kilo`/`mile`) is NOT recoverable from the type string at render time.

## The fix — per-element scale must come from EMIT (two parts)
The type string can't carry the scale (it's at-reference), so emit must provide per-element scale info that
the render consumes recursively. Mirror wasm's `const_value_ast_scaled` (lower.rs ~14089): per element, by
inner type — Float `×num/den` (f64/f32, IEEE rounds), Int `×num/den` (own width, truncates), Rational
cross-mult exact (`num/den` as a Rational, normalized). Recurse into Tuple holes, Option/Result payloads,
nested tuples. Label each at `unit.at_reference()`.

### Option A (preferred): a PER-PATH scale note map
Emit, for a compound-Qty result, a note listing each Qty leaf's PATH + its unit value-form + scale:
`// cdz-qty-at[ident]: <path> <value-form> <num>/<den>` (one per Qty leaf, path = the tuple/payload index
route, e.g. `0`, `1`, `0.0` for nested). `cdz_render_expr` already walks `path` as it descends
(`cdz_render_at(inner_ty, render_path, …)`), so at each Qty leaf it looks up the note for the CURRENT path
and applies that unit-form + scale — instead of only the top-level `unit_form`/`unit_scale`. Scale-1 leaves
emit no entry (raw render, as today). This keeps the wasm-side untouched and localizes the change to
`mod.rs` (emit the per-leaf notes by walking the result type for Qty leaves) + `cdz-rust-render` (thread a
`&HashMap<path, (value_form, scale)>` and consult it in the Qty arm instead of the single top-level Option).

### Option B: bake the scaled value into the emitted VALUE
Instead of a render-time multiply, scale at EMIT (the const-fold already has the magnitude + unit). Emit the
already-scaled magnitude + at-reference unit directly, so the render needs NO scale knowledge. Cleaner render
but changes what the rust program COMPUTES (the raw magnitude is what `Qty.value` returns internally — must
NOT change that; only the boundary DISPLAY scales). Risk: conflates internal value with display. REJECT
unless the compound render path is display-only (it is — it's the gate value-form). Revisit if A is fiddly.

## Sequencing
- BLOCKED on my S2-consumer MR (`45de7323b`) landing first (per-commit cadence; don't stack a large fix on
  an unlanded commit that could be reworked on reject).
- After S2 lands: implement Option A, gate (rust 3-4 todo→pass in 18-units, 0 regress; rust-async twin;
  wasm untouched), coordinate the baseline with v-quantity (corpus-bugfix routed them the missing-baseline
  hygiene — they add wasm=pass/rust=todo/rust-async=todo; my fix flips the rust/async todos→pass).
- v-quantity is the consultant on scaled oracle values + scale factors per unit (mile=201168/125 m, etc.).

## Coordination
- v-quantity: units semantics + oracle values (offered to verify emit against the wasm oracle).
- corpus-bugfix: owns the baseline hygiene for the unbaselined cases; will pin the scale-fold-into-holes fix.
- The `cdz-rust-render` crate was extracted by v-cdz-tooling — the render-side edit is in that crate; ping
  them on the seam (the per-path note threading is a new param to `cdz_render_expr`/`cdz_render_at`).
