# PR#724 review comments — rust Qty per-element scale docs stale (Option/Result payload now supported via `?N` path)

Mirrored from GitHub PR review comments (Copilot), ids `3619816583`, `3619816612`, `3619816626`.
PR: https://github.com/camshaft/cadenza/pull/724 (merged; fix still belongs on trunk)
Locations:
- `implementation/seed/crates/rcdzc/src/backend/rust/mod.rs:103`
- `implementation/seed/crates/cdz-rust-render/src/lib.rs:411`
- `implementation/seed/crates/cdz-rust-render/src/lib.rs:507`

## Comments (verbatim)

- (id 3619816583, mod.rs:103) "The doc comment says per-element quantity scaling is limited to
  Tuple/Record and that Option/Result payload quantities are a follow-up, but the implementation
  below now handles `Option`/`Result` by adding `?N` path segments. This comment should be updated
  so readers don't incorrectly assume Option/Result payload scaling is unsupported."
- (id 3619816612, lib.rs:411) "The `cdz_qty_at` doc comment describes `<path>` as only a `.i` route
  (tuple/record). But the Rust backend now emits paths for Option/Result payloads using `?0`/`?1`
  segments (e.g. `?0`, `0?0`), so the documentation should mention that path grammar."
- (id 3619816626, lib.rs:507) "The `logical_path` parameter comment says non-tuple/record descents
  forward the path unchanged and that sums are out of scope, but the implementation now extends
  paths for `Option`/`Result` payloads (`?0`/`?1`). The comment should reflect that so future
  changes don't accidentally break the emit/render agreement."

## Liaison verification (CONFIRMED on trunk)

All three are stale docs against the SAME feature landing (`0327858d4`, "rust backend Qty
compound-value per-element display-scale (tuple/record/Option/Result)"):

1. `backend/rust/mod.rs` — `collect_qty_scale_paths` doc (lines ~101-103) says "SCOPE (this slice):
   TUPLE + RECORD holes only … A Qty inside an Option/Result payload … is a FOLLOW-UP", but the fn
   body now has `Ty::Sum { name: "Option"|"Result" }` arm (lines ~145-153) emitting `?N` segments
   (Option payload `?0`, Result Ok `?0` / Err `?1`, nested composes e.g. `0?0`).
2. `cdz-rust-render/src/lib.rs:407-411` — `cdz_qty_at` doc describes the path as only the `.i` route
   (`0`, `1`, `0.1`); should mention the `?N` segment grammar.
3. `cdz-rust-render/src/lib.rs:504-508` — `logical_path` param comment says "sum/list/newtype are
   out of this slice's per-element scope" + "forwarded unchanged by every other descent", but
   Option/Result payloads now extend the path with `?N`.

Fix: update all three docs to describe the `?N` Option/Result path grammar (user sums + lists remain
out of scope — verify that's still true). Doc-only, hash-neutral, no behavior change.
Owner: v-rust-backend (commit `0327858d4`). Routed as a note.
