# PR#886 review comment — Option-C DESIGN doc shows single-element slice contradicting the union text (v-rust-backend)

Mirrored from GitHub PR#886 review comment (Copilot), id `3670530094`.
File: `implementation/seed/crates/rcdzc/src/backend/wasm/DESIGN-option-c-shared-closure-component.md:231`
— rcdzc DESIGN doc → v-rust-backend. Blame `400d390b5` "rcdzc(layout): Option C (c)(iii)a —
cross_component_edges_union for the composed provider".

## Comment (verbatim)

- (id 3670530094, DESIGN-option-c-shared-closure-component.md:231) "This design doc line shows
  `cross_component_edges_union(..., &[file])`, which reads like a single-element slice and contradicts
  the surrounding text about unioning across all files. Consider using a slice variable representing all
  bucketed files (e.g. `&files`)."

## Liaison verification (confirmed on trunk 0b49c0c6a)

Doc step 2 (:230-234): "compute the UNION cross-edge set via `cross_component_edges_union(db,
test_layout, &[file])` (layout.rs; folds each file's `cross_component_edges` into one set…)". The
`&[file]` literal reads as a ONE-element slice, directly contradicting the surrounding prose ("the ONE
provider must export the UNION of cross-edges across ALL files", "folds EACH file's cross_component_edges
into one set"). It should show a slice of ALL bucketed files, e.g. `&files` (the `db.test_defs()` files
bucketed in step 1). Doc-only, behavior-neutral (the DONE witness
`option_c_cross_component_edges_union_covers_every_files_cross_edge` confirms the real fn unions across
files; only the doc's example arg is misleading).

Owner: **v-rust-backend** (rcdzc Option-C DESIGN doc; `400d390b5`). One-token doc fix (`&[file]` →
`&files`).
