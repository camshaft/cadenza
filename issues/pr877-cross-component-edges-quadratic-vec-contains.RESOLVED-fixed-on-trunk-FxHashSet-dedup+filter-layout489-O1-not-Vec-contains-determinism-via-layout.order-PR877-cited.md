# PR#877 review comment — cross_component_edges O(N^2) Vec::contains (v-rust-backend)

Mirrored from GitHub PR#877 (OPEN staging batch) review comment (Copilot), id `3665418099`.
File: `implementation/seed/crates/rcdzc/src/layout.rs:489` — `rcdzc` crate → v-rust-backend's lane.
Blame `d8ee3fa1b` "rcdzc(layout): Option C increment (b)(i) — cross_component_edges".

## Comment (verbatim)

- (id 3665418099, layout.rs:489) "`cross_component_edges` deduplicates and orders edges using
  `Vec::contains`, which makes this O(N^2) in the number of call edges / reachable defs and repeats the
  linear scan again when filtering `layout.order`. Using an `FxHashSet` for membership keeps determinism
  (by filtering in `layout.order` order) while avoiding quadratic behavior."

## Liaison verification (confirmed on trunk ec6fba606)

`cross_component_edges` (fn ending ~491): builds `edges: Vec<usize>` with `!edges.contains(&c)` guard
inside a nested loop over `own` × `callees` (O(edges) per push → O(N^2) dedup), then RE-scans with
`.filter(|d| edges.contains(d))` over `layout.order` (another O(N^2)). Suggested fix is sound: use an
`FxHashSet<usize>` for membership; determinism is PRESERVED because the returned order comes from
iterating `layout.order` (source-fixed) and filtering by set membership — the reproducible-derivation
contract pinned right below (`//= reproducible-derivation.md#codegen-order-is-source-determined`) is
satisfied by ordering via `layout.order`, not by the set. Perf-only, behavior-neutral (same edge set,
same order). Note the fn is on the Option C cross-component-interface path (shared workstream), but the
CODE is in rcdzc → v-rust-backend owns it.

Owner: **v-rust-backend** (`rcdzc/src/layout.rs`; `d8ee3fa1b`). Swap the dedup `Vec::contains` for an
FxHashSet, keep the `layout.order` filter for deterministic ordering.
