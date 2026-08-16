# Site-6 through-block fold (adv-69, ff76dd2e5) — capture/order attack sweep, 2026-08-10

Target: the freshly-landed Site 6 commuting conversion (float pure let-wrappers out of a
branch-perform let-init). Attack surfaces: name capture on float, trap ordering, chained
wrapped inits, depth-2 wrappers, match-scrutinee interaction.

All GREEN x3 (wasm/rust/rust-async) — pin candidates for a future batch:
- s6a: wrapper binding SHADOWS an outer binding the body reads (capture hazard) — 404
- s6b: wrapper shadows an outer binding a LATER outer init reads — 10304
- s6c: TWO wrapped branch-performing inits, second wrapper reads first's value — 345/1508
- s6d: floated wrapper init can TRAP (/ 100 n) — order preserved, n=0 traps div-by-zero
- s6e: depth-2 pure wrappers, inner reads outer, conditional reads both — 34/174
- s6f: floated conditional as MATCH scrutinee (Site 5 handoff) — 503/305/9
  (hand-math slip caught pre-filing: first dispatch returns the SEED, arm literal 4 not 3)

No counterexample found — Site 6's pure-only peel (reaches_any_perform) + deep_fresh_copy
rebuild holds on capture, order, and trap faces.
