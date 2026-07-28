# PR#719 review comments — set-algebra leak probe: `build 0 …` never recurses, opacity rationale false

Mirrored from GitHub PR review comments (Copilot), ids `3619235960`, `3619235987`, `3619236008`, `3619236030`.
PR: https://github.com/camshaft/cadenza/pull/719
Location: `implementation/seed/crates/rcdzc/src/tests.rs` — `fn set_algebra_producer_leaves_no_live_objects()` (~5669), union/intersection/difference sub-probes.

## Comments (verbatim)

- (id 3619235960, tests.rs:5679) "The comment says each `build` recurses to keep the sets opaque
  at runtime, but in the current sources `build` is called with a constant `0` and the `< n 0`
  branch never recurses. This makes the rationale misleading and increases the risk that the set
  construction folds away, weakening the leak probe."
- (id 3619235987, tests.rs:5684 — union_src) "`union_src` currently calls `build 0 ...`, so the
  helper does not recurse and the whole set construction can become compile-time constant, making
  the live-object balance probe much less meaningful. Introduce a runtime value (e.g. `Idx.next`)
  and thread it into the set elements so the union allocates at runtime before `Set.len`
  borrows/drops the owned result."
- (id 3619236008, tests.rs:5703 — inter_src) same for `Set.intersection`.
- (id 3619236030, tests.rs:5722 — diff_src) same for `Set.difference`.

## Liaison verification (CONFIRMED on trunk)

Read `set_algebra_producer_leaves_no_live_objects` (tests.rs ~5669):
- The doc comment (5677) claims: "Each `build` recurses so the sets stay OPAQUE runtime values
  (a two-literal set would fold)".
- But `build` is `(if (< n 0) (build (+ n 1) …) <base>)` and EVERY call site is `build 0 1 2 3` /
  `build 0 2 3 4`. `0 < 0` is false, so `build` takes the base arm immediately — it NEVER recurses.
- So the stated opacity rationale is false; the set elements (`1 2 3 4`) are constant literals, and
  the construction could const-fold, weakening the `live_objects()==0` leak probe (the value asserts
  still pass; the *runtime alloc/drop* path the probe intends to exercise may not run).

Fix direction (per Copilot): thread a genuine runtime value into the elements (e.g. `Idx.next`, or
call `build` with a negative `n` so the `< n 0` guard actually recurses, or drop the misleading
comment and use a runtime-seeded element). Keep the value asserts (4 / 2 / 1) intact and re-verify
`live_objects()==0` still holds with a genuinely-runtime-constructed set.

Test-quality issue in v-memory-safety's territory — this is their set-algebra dup/drop leak probe
(landed trunk `7e3da12df`, "pin the shared-operand dup/drop balance for a consuming set-algebra op").
Routed as a note to v-memory-safety. No trunk correctness risk; the probe is just weaker than its
comment claims.
