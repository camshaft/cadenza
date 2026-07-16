# PR review comment — mirrored from GitHub PR #459 (Copilot inline)

- **PR:** #459 "fleet: batch 88+89 corrected (…, cad R5)" (OPEN at triage; file on trunk)
- **File:** `guide/src/cad/index.ts:260` (`meshFromSolid` / `toManifold` `Emptyr`)
- **Reviewer:** Copilot (automated)
- **Comment id:** 3593493145
- **Link:** https://github.com/camshaft/cadenza/pull/459#discussion_r3593493145

## Comment (verbatim)
> `meshFromSolid` currently routes every parsed solid through manifold meshing, and `Emptyr` is represented in `toManifold` as a zero-size cube. In the Rust cdz-cad driver, `Emptyr` becomes an actually-empty manifold/mesh (no triangles). Returning a degenerate cube risks producing non-empty geometry (or odd bounds) for an empty solid. Consider short-circuiting the empty case to return empty buffers.

## Liaison triage — CONFIRMED plausible
The guide's browser CAD path (`meshFromSolid` → `toManifold`) represents `Emptyr` as a ZERO-SIZE cube,
but the Rust cdz-cad driver makes `Emptyr` an actually-empty manifold/mesh (no triangles). A degenerate
zero-size cube can yield non-empty geometry or odd bounds for what should be an empty solid — a
guide-vs-native driver inconsistency (the two CAD backends should agree, like the Cuber/Cylinderr
size reconciliation). FIX: short-circuit `Emptyr` in `meshFromSolid` to return empty buffers (no
triangles), matching the Rust driver. CAD territory (v-cad owns the geometry model; guide/src/cad).
Fix on `trunk`. Quote + link in queue file.
