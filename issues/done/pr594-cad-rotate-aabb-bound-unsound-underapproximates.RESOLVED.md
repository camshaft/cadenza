# pr594 — cad exact.cdz: Rotate AABB bound is UNSOUND (under-approximates); 2 tests pin the bug (3 Copilot)

Mirrored from GitHub PR #594 review comments (Copilot).
PR: https://github.com/camshaft/cadenza/pull/594 (16-MR publish batch, MERGED to trunk)
Files: `implementation/cad/src/exact.cdz` (bound + 1 test) + `implementation/cad/src/helpers.cdz` (1 test).
All 3 VERIFIED against `git show trunk` — v-cad territory. One coherent bug + its two pinning tests.

## THE BUG — id 3608845833 (exact.cdz:276) — max|coord| does NOT enclose arbitrary rotations
> `aabbr-corner-max-abs` is used to bound `Rotate`, but taking only the maximum absolute coordinate is
> not sufficient to enclose arbitrary rotations about the origin. Counterexample: the cube point (1,1,0)
> has max|coord|=1 but after a 45° rotation around z it reaches y=√2 (>1), so `[-m,m]^3` with
> m=max|coord| does not enclose the rotated solid. A sound exact (Rational) conservative bound is to use
> an L1 radius upper bound `m = ax + ay + az`, where `ax = max(|lx|,|hx|)` etc; then each rotated
> coordinate is <= ||p||₂ <= ||p||₁ <= m.

VERIFIED: `def aabbr-corner-max-abs(lo, hi)` (exact.cdz:274) = `rmax` over per-axis `rabs(lx..hz)`, and its
doc literally claims "`[-m, m]^3` encloses ANY rotation (exact, conservative)". That claim is FALSE:
(1,1,0) has max|coord|=1 but a 45°-about-z rotation sends it to (0, √2, 0), y≈1.414 > 1 — OUTSIDE
`[-1,1]^3`. So `Rotate`'s bounding box is UNSOUND (too small) — a real geometry correctness bug (a
bounding box that fails to enclose the solid breaks any downstream culling/containment). Copilot's fix is
sound and exact-Rational-friendly (no √, no Float): `m = ax + ay + az` (L1 radius), since for a rotation
about the origin ‖Rp‖₂ = ‖p‖₂ ≤ ‖p‖₁ ≤ ax+ay+az. NOTE: Cadenza has no Float64 trig / sqrt (v-cad works
in exact Rational), so the L1 bound is the right conservative choice — do NOT try an exact rotated-corner
box (needs trig).

## PINNING TESTS to update after the fix
- id 3608845841 (exact.cdz:1032) `rotate-bounds-conservatively-encloses`: asserts a 2×2×2 cube's Rotate
  box stays size-2 `[-1,1]^3` — that's the UNDER-approximation. After the fix, the sound bound for a cube
  spanning [-1,1]^3 is L1 m = 1+1+1 = 3 → box size 6. Update the assertion + comment.
- id 3608845845 (helpers.cdz:142): asserts rotate-z(45) of a bar (x∈[9,11], y,z∈[-1,1]) has width 22
  (= 2·max|coord| = 2·11). After the fix the L1 bound is 11+1+1=13 → width 26. Copilot computed this.
  Update the expected width + comment.

## Owner
`implementation/cad/*` = v-cad. The bound fix + both test updates land together (else the tests fail).

---
RESOLVED (corpus-bugfix 2026-07-19, verified on trunk dcc81d629): the unsound Rotate-AABB under-approximation
is FIXED in implementation/cad/src/exact.cdz. `aabbr-corner-max-abs` (exact.cdz:278) now returns the L1 sum
`mx + my + mz` (per-axis max-abs summed), NOT the old `rmax(...)` max|coord|. Doc (272-276) explains: "sqrt has
no exact Rational form, so we use the L1 upper bound mx+my+mz which SOUNDLY ENCLOSES the rotated shape for ANY
angle" and explicitly cites "the reviewer-flagged under-approximation. The L1 sum fixes it." The pinning test
was RENAMED conservatively→soundly (`rotate-bounds-soundly-encloses`, exact.cdz:1035) reflecting the corrected
bound; helpers.cdz rotate tests consistent. Exactly Copilot's fix (‖Rp‖₂ ≤ ‖p‖₁ ≤ ax+ay+az, exact-Rational,
no trig/sqrt). Owner (v-cad) resolved — no corpus-bugfix action.
