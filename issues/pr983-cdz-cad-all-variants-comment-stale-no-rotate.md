# PR#983 review comment — cdz-cad ALL_VARIANTS comment says "no Rotate" but the fixture now includes Rotate (v-cad)

Mirrored from GitHub PR#983 review comment (Copilot), id `3694801901`.
File: `implementation/seed/crates/cdz-cad/src/lib.rs:535` — cdz-cad crate → v-cad. Blame `3693d9543`
"cad(cdz-cad): make ALL_VARIANTS a true grammar-completeness guard (all 13 Solid heads + every PathSeg)".

(The PR's OTHER Copilot comment, id 3694801912 on issues/pr979-…md, was a mirror-provenance duplicate of
the PR#940 ruling — DISMISSED, see ledger.)

## Comment (verbatim)

- (id 3694801901, cdz-cad/src/lib.rs:535) "This comment is now inaccurate: the grammar-completeness guard
  string below includes a `Rotate` constructor, but this line still says 'no Rotate'. Update the comment
  so it matches the current ALL_VARIANTS fixture (and avoid claiming the string is 'captured' if it's now
  curated to cover all variants)."

## Liaison verification (confirmed on trunk a42c3f91a)

The ALL_VARIANTS test-fixture comment (lib.rs:534-540): line 534 "The EXACT text cdz-run renders for a
Solid (CAPTURED from the live compiler), covering every variant. Rational leaves `n/d`; **no Rotate (no
exact rotation)**." But the fixture was reworked (`3693d9543` "true grammar-completeness guard — all 13
Solid heads + every PathSeg") and line 539 now explicitly says "Right subtree adds **Rotate**→
ExtrudeLinear(PathProfile) and Mirror→Revolve(Circle)". So the ":534 no Rotate" is stale — the fixture NOW
includes Rotate. Two nits: (a) drop/flip "no Rotate"; (b) "captured from the live compiler" is now
inaccurate — it's CURATED to cover all 13 heads + every PathSeg (a completeness guard), not a raw live
capture. Reword both to match the curated all-variants reality. Comment-only, behavior-neutral.

Owner: **v-cad** (`implementation/seed/crates/cdz-cad/src/lib.rs`; `3693d9543`). Update the stale "no
Rotate" + "captured" comment to the curated all-13-heads/every-PathSeg fixture.
