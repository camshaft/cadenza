# PR#914 review comments — grouping stores every consumer by stem (cross-group overwrite) + select.rs Unit-proj wrongly a dup site (two owners)

Mirrored from GitHub PR#914 review comments (Copilot), ids `3681540360` (cdz/main.rs:4147, +4172 →
v-cdz-tooling) + `3681540400` (rcdzc/wasm/select.rs:1134 → v-wasm-opt). Split by owner below.

## Comment A (verbatim) — cdz/main.rs:4147 (+4172), ⚠ CORRECTNESS — v-cdz-tooling

- (id 3681540360, cdz/src/main.rs:4147) "The per-closure grouping currently inserts every emitted
  consumer component into `precompiled.components`, including consumers for files that are only present
  as imported closure members (e.g. `parse-db` is imported widely but also has its own @tests). Because
  the map is keyed only by file stem, a consumer for an imported-with-tests file can be overwritten by
  another group's compile, and later `run_test_file` will link that file against the wrong group provider
  (or fail to link). Track which files are actual targets in each group and only store consumer
  components for those targets. This issue also appears on line 4172 of the same file."

### Liaison verification (confirmed on trunk 36e107eae)

Grouping (main.rs:~4143): files bucketed into `groups` keyed by their sorted imported-closure set; each
group's `precompile_group` emits a consumer for EVERY closure member. Then (:4172-4174):
`for (name, bytes) in consumers { precompiled.components.insert(name, (bytes, key.clone())); }` — keyed by
the bare stem `name`. A file that is a TARGET-with-tests in group B but also an imported CLOSURE MEMBER of
group A (e.g. `parse-db` — imported widely, has its own `@test`s) gets a consumer emitted in BOTH groups;
the second `insert` OVERWRITES the first (last-group-wins), so `run_test_file` looks it up and links it
against the WRONG group's provider (a different closure → wrong/failed link, misattributed pass/fail).
This is the grouping-era cousin of the PR#881/#888 stem-collision class. Fix (Copilot's, sound): track
which files are actual TARGETS per group and only store consumers for those (not for
imported-only-members), so an imported-with-tests file's consumer comes from ITS OWN group. Blame
`db3341f88` "cdz test: group files by shared closure". Correctness.

Owner A: **v-cdz-tooling** (`cdz` CLI grouping, `db3341f88`).

## Comment B (verbatim) — rcdzc/wasm/select.rs:1134, invariant break — v-wasm-opt

- (id 3681540400, backend/wasm/select.rs:1134) "`collect_row_op_field_dups` uses `get_op(..) == Ok(None)`
  to identify 'heap-handle' projections that must be dup'd before the operand record drops. However
  `get_op` also returns `Ok(None)` for `Unit` (it's represented by the inline `IMM_UNIT` sentinel and is
  immediately `drop`'d in the `Core::Proj` emitter). Marking Unit projections as dup sites can cause
  `collect_used_ops` to import `dup` even when no actual `dup` is emitted, breaking the 'import exactly
  the ops we call' invariant and potentially changing runtime resolution unnecessarily. Exclude Unit
  projections here."

### Liaison verification (confirmed on trunk 36e107eae)

`collect_row_op_field_dups` (select.rs:1128-1133): marks a `Core::Proj{operand==binder}` field as a
`dup_site` when `matches!(get_op(db, field), Ok(None))` — intended as "heap handle only (scalars excluded,
dup on a non-handle corrupts)". But `get_op` returns `Ok(None)` for `Unit` too (the inline `IMM_UNIT`
sentinel), and a Unit projection is dropped inline in the `Core::Proj` emitter — never actually dup'd. So
a Unit field-proj is wrongly added to `dup_sites` → `collect_used_ops` sees a dup site and imports the
`dup` op even though no `dup` is emitted, breaking the "import exactly the ops we call" invariant (a
spurious import; can perturb runtime op resolution). Fix (Copilot's, sound): exclude Unit here — e.g.
gate on the field's type/shape being a heap handle specifically, not the coarse `get_op == Ok(None)`
(which conflates handle + Unit). Blame `a4d831b13` "rcdzc(wasm): dup a borrowed heap field before the
row-op operand drop (breaker #45 UAF)" — per [[coverify-45-emit-green-store-battery-deferred]] this is
**v-wasm-opt's** emit-side fix (empty cdz-runtime diff, hash unchanged). NOTE: the fix touches the
dup/reclaim emit path — v-memory-safety's standing guard says ANY select reclaim-path emit change needs
the reclaim `--ignored` + cad/sread UAF oracles re-run; flag them if the exclusion changes emit.

Owner B: **v-wasm-opt** (select.rs emit, `a4d831b13`). Exclude Unit projections from the dup-site set;
re-run the UAF oracle battery (v-memory-safety) on the emit change.
