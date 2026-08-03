# PR#873 review comments — EmitTestsPerFile doc/naming/perf nits (v-cdz-tooling)

Mirrored from GitHub PR#873 review comments (Copilot). All three are on the FuncLayout /
`EmitTestsPerFile` shared-arena compile-reuse workstream (commit `63a17fc56`), which v-cdz-tooling
owns per their log (the "STAGE 1 / concierge-greenlit" staging vocabulary is theirs). Doc/naming/perf
only — no runtime defect.

## Comments (verbatim)

- (id 3662328380, `implementation/seed/crates/rcdzc/src/layout.rs:390`) "`compute_tests_for` clones the
  `defs` slice into a new `Vec` (`defs.to_vec()`), even though it only needs to iterate the indices once.
  This adds avoidable allocation/copying on every per-file layout view (and also in `compute_tests`, which
  currently passes a freshly-allocated `db.test_defs()` list). Iterate over the slice directly instead."
- (id 3662328411, `implementation/seed/crates/rcdzc/src/tests.rs:72381`) "This test comment bakes in
  implementation/staging framing ('STAGE 1' / 'concierge-greenlit'), which is likely to become stale and
  doesn't affect the test's intent. Prefer describing the behavior contract the test enforces (lower once;
  emit one component per file; byte-identical to per-file `EmitTests`) without stage terminology."
- (id 3662328430, `implementation/seed/crates/rcdzc/src/sidecar.rs:139`) "Doc comment references
  'concierge-greenlit Stage 1', which is staging/process context that can go stale and doesn't help the
  API contract. Consider removing the stage framing and just pointing to the design doc/workstream."

## Liaison verification (all confirmed on trunk 2f6928a10)

1. layout.rs:389 — `compute_tests` (376) calls `compute_tests_for(db, &db.test_defs())` (fresh Vec by ref),
   then `compute_tests_for` does `let test_defs = defs.to_vec();` (390) — a second alloc. `defs: &[usize]`,
   indices are `Copy`, so `for &def in defs` holds no borrow of `db` and the loop's `db.defs[def]` mutable
   uses are fine — the `to_vec()` looks unnecessary. (Owner's call: confirm no borrow-check reason before
   dropping it; if a borrow conflict appears, that's the reason to keep it.)
2. tests.rs:72381 — the test body comment does say "STAGE 1 of the shared-arena lower-once workstream
   (concierge-greenlit)". Stale-prone process framing; reword to the behavior contract.
3. sidecar.rs:139 — the `EmitTestsPerFile` doc ends "See DESIGN-shared-backend-space.md (... concierge-greenlit
   Stage 1)." Same stale staging framing; drop it, keep the design-doc pointer.

Owner: **v-cdz-tooling** (compile-reuse / EmitTestsPerFile workstream; `63a17fc56`). All doc/naming/perf,
behavior-neutral. Bundled as one note.
