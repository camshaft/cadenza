# PR#880 review comments — layout.rs boundary.contains hot-path + unordered boundary_hits (v-rust-backend)

Mirrored from GitHub PR#880 review comments (Copilot), ids `3667923137` (:684, also :735) + `3667923181`
(:576). Both `implementation/seed/crates/rcdzc/src/layout.rs` = rcdzc crate → v-rust-backend. Blame
`d59157ddc` "rcdzc(layout): Option C (c)(i) — compute_tests_consumer excludes cross-edges from the
emission set" (same Option-C cross-component work as PR#877's cross_component_edges route).

## Comments (verbatim)

- (id 3667923137, layout.rs:684) "`finish_layout_bounded` is now used by the normal `finish_layout` path
  with an empty `boundary`, but this `boundary.contains(&c)` check still runs for every discovered
  callee. Since `boundary` is usually empty, guard the check to avoid an extra hash lookup on the hot
  reachability path. This issue also appears on line 735 of the same file."
- (id 3667923181, layout.rs:576) "`compute_tests_consumer` returns `boundary_hits` as a `HashSet`, but
  the doc comment relies on later building `extern_order` in the provider's canonical cross-edge order so
  import indices match export indices. Returning an unordered set makes it easy for a caller to
  accidentally iterate hits and construct nondeterministic / mismatched import order; consider returning
  hits as a `Vec<usize>` already in canonical boundary order (or take an ordered `&[usize]` boundary and
  return hits in that same order)."

## Liaison verification (both confirmed on trunk a4430be8d)

1. layout.rs:681 (+ sibling 735) — `if boundary.contains(&c) { boundary_hits.insert(c); continue; }`
   inside the per-callee loop of both the reachability worklist and the lifted-body walk. The doc right
   above says "Empty boundary → the condition never fires → byte-identical to the ordinary layout" — so
   on the normal `finish_layout` path `boundary` IS empty yet the hash lookup runs for every callee.
   Guarding with `!boundary.is_empty() && boundary.contains(&c)` skips it. Perf micro-opt on the hot
   path, behavior-neutral (empty-set contains is always false anyway).
2. layout.rs:571-576 — `compute_tests_consumer` returns `(Layout, HashSet<usize>)` for `boundary_hits`.
   The doc (lines 566-568) hinges on the consumer's import index matching the provider's export index via
   "the provider's canonical cross-edge order" (the v-wasm-opt index-agreement invariant). An UNORDERED
   HashSet return leaves ordering to the caller and invites a nondeterministic import order (a real
   invalid-module / index-mismatch risk class, per the reproducible-derivation + v-wasm-opt index
   invariants). Suggest returning `Vec<usize>` already in canonical boundary order, or taking an ordered
   `&[usize]` boundary and returning hits in that order. API-safety / determinism — owner's judgment on
   whether the current callers already impose the order downstream (if so it's latent, not live).

Owner: **v-rust-backend** (`rcdzc/src/layout.rs`, Option-C `d59157ddc`). Bundled as one note (both in the
same fn family). Perf + determinism-hardening, behavior-neutral.
