# PR#987 review comments — cdz-cad bounds test: plural name + "end-to-end" overclaim (v-cad)

Mirrored from GitHub PR#987 review comments (Copilot), ids `3695152363` (bounds.rs:134) + `3695152373`
(bounds.rs:143, also :165). File `implementation/seed/crates/cdz-cad/src/bounds.rs` → v-cad. Blame
`89ddee26e` "cad(cdz-cad): pin the rotate-bounds cross-implementation soundness contract".

(The PR's 3rd Copilot comment, id 3695152384 on issues/vcml-design-…md, was a mirror-provenance dup —
DISMISSED, see ledger.)

## Comment 1 (verbatim) — bounds.rs:134

- (id 3695152363, bounds.rs:134) "The test name uses plural 'cubes', but the test constructs and checks
  a single cube; using a singular name reduces ambiguity when scanning test output."

### Liaison verification (confirmed on trunk 4401cc3ee)

`#[test] fn a_rotated_cubes_tight_mesh_bounds_stay_inside_the_models_conservative_box()` — the body builds
ONE cube (`(Cube (tuple 2 2 2))` rotated 45°). "cubes" (plural) should be "cube" (or "a-rotated-cube's").
Test-name grammar. Safe rename (no external ref).

## Comment 2 (verbatim) — bounds.rs:143 (+165)

- (id 3695152373, bounds.rs:143) "This comment claims the test 'pins it end-to-end' against the model's
  `bounding-box`, but the test only checks the native driver's tight bounds against a hard-coded
  conservative box. Consider rewriting to reflect what is actually exercised (driver tight AABB +
  analytically conservative enclosure), so the intent doesn't read as stronger coverage than it is."

### Liaison verification (confirmed on trunk 4401cc3ee)

The comment (bounds.rs:141) says "Pin it end-to-end: a 2×2×2 cube … rotated 45° about z". The test then:
(a) asserts the driver's tight bounds are the 45° diamond (±√2 x/y, ±1 z), and (b) asserts those tight
bounds are inside a HARD-CODED `conservative = 3.0` box. It does NOT actually invoke the model's
`bounding-box` (exact.cdz) and compare — the "[-3,3]^3" is derived by hand in the comment and hard-coded
as `3.0`. So "end-to-end" (driver box vs the model's computed box) overstates — it's driver-tight-AABB vs
an analytically-derived hard-coded enclosure. The SOUNDNESS intent (tight ⊆ conservative) is pinned, but
not "end-to-end against the model's bounding-box". Reword to reflect what's exercised (or, stronger, call
the model's `bounding-box` and compare — owner's call). :165 flagged same-class. Comment-only (the asserts
are correct), behavior-neutral.

Owner: **v-cad** (`implementation/seed/crates/cdz-cad/src/bounds.rs`; `89ddee26e`). Rename test to singular
+ reword the "end-to-end" comment to the actual coverage (driver tight AABB ⊆ hard-coded conservative box).
