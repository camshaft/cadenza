# PR#882 review comments — cross-edge extern-import index shift (⚠ correctness) + stale compute_tests_consumer doc (v-rust-backend)

Mirrored from GitHub PR#882 review comments (Copilot), ids `3668893522` (mod.rs:543, ⚠ correctness) +
`3668893555` (layout.rs:606, doc). Both `rcdzc/*` = v-rust-backend, Option-C cross-component work.

## Comment 1 (verbatim) — ⚠ CORRECTNESS, mod.rs:543

- (id 3668893522, backend/wasm/mod.rs:543) "`layout.cross_edge_import` positions are treated as indices
  into `extern_order`, but this block appends cross-edge `ExternImport`s to `extern_imports` and later
  rebuilds `extern_order` from that vector. If any peer-bound effect imports are present, they will
  appear before the cross-edge imports and shift indices, causing `Lir::CallExternImport(pos)` to call
  the wrong import (or validate incorrectly). Insert the cross-edge imports into `extern_imports` at
  their intended `pos` so the final `extern_order` matches the precomputed positions."

### Liaison verification (confirmed on trunk 6225686a8; latent-vs-live is owner's call)

- mod.rs:490-503: FIRST, peer-bound escaping effects (`db.effect_bindings`) are moved into
  `extern_imports` (push at 494) — these occupy positions `0..K`.
- mod.rs:517-544: THEN the cross-edge block appends its `ExternImport`s (push at 532), ordered by
  `cross_edge_import`'s precomputed `pos` (`by_pos.sort_unstable()`), so they land at `K..K+M`.
- mod.rs:587: `extern_order` is rebuilt from the whole `extern_imports` vector in push order.
- BUT `cross_edge_import[def] = pos` was computed independently in `compute_tests_consumer` as a 0-based
  index into the provider's export order (0..M). A `Lir::CallExternImport(pos)` resolves against the
  FINAL `extern_order`, where the cross-edge at intended `pos` now actually sits at `K+pos`. So if BOTH
  a peer-bound effect import (K>0) AND cross-edges are present, every cross-edge call is off by K → wrong
  import called / validation failure (an invalid-module or wrong-callee class bug — the v-wasm-opt
  index-agreement invariant this code is explicitly trying to preserve, per the :518 comment).
- Blame `fc673bb19` "Option C (c)(ii-c) — consumer emit populates extern_imports from cross_edge_import".

LIVE vs LATENT: today a per-file `@test` CONSUMER layout may not also carry a bound escaping effect (the
consumer path is the shared-closure test import; bound effects come from `(bind …)` in the program). If a
consumer can never have K>0, this is LATENT. But the fix is cheap and removes the footgun: either offset
the cross-edge `pos` by the count of already-present (peer-bound) extern imports when consulting
`extern_order`/emitting `CallExternImport`, or insert cross-edges at their intended absolute `pos`.
Owner (v-rust-backend) knows whether a consumer + bound-effect combination is reachable. Flagged as
correctness for that reason — please confirm live/latent, not just reword.

## Comment 2 (verbatim) — DOC, layout.rs:606

- (id 3668893555, layout.rs:606) "The doc comment immediately above `compute_tests_consumer` still
  describes the old API (returning `(Layout, boundary_hits)` and having the caller build `extern_order`
  from hits). The function now returns only `Layout`, takes `provider_edges` + `closure_iface`, and
  builds the import mapping itself, so that comment should be updated to match the current
  behavior/signature."

### Liaison verification (confirmed on trunk 6225686a8)

Doc lines 596-602 say "Returns the layout + the cross-edge defs that were actually HIT … the caller
builds `extern_order` from them". But the CURRENT signature (602-607) is
`pub fn compute_tests_consumer(db, test_defs: &[usize], provider_edges: &[usize], closure_iface: &str) ->
Result<Layout, Reject>` — returns only `Layout`, takes `provider_edges` + `closure_iface`, and populates
`cross_edge_import` internally (this is the landed PR#880/#882 API change). Stale doc; reword to match.
Blame `2ec6e57b0` "Option C (c)(ii-a)". Doc-only.

Owner: **v-rust-backend** (both `rcdzc/*`, Option-C cross-component). Bundled; comment 1 is correctness
(confirm live/latent), comment 2 is doc-only.
