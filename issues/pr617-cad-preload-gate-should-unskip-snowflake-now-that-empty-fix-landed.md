# pr617 — check-cad-preload.mjs still skips parametric-snowflake though the empty-Solid fix has LANDED

Mirrored from GitHub PR #617 review comment (Copilot), id 3609646154.
PR: https://github.com/camshaft/cadenza/pull/617 (7-MR publish batch)
Location: `guide/scripts/check-cad-preload.mjs:319` (`MESH_SKIP`)

## Reviewer comment (verbatim)
> The new headless visible-geometry gate currently skips `parametric-snowflake`, but this PR also fixes
> the Empty→Manifold mapping to a composing empty (M.union([])). With that fix in place, keeping the skip
> reduces the gate's coverage and could let a blank-snowflake regression slip again; include the snowflake
> in the mesh-check loop.

## VERIFIED (git show trunk) — the skip is now genuinely stale
`const MESH_SKIP = new Set(["parametric-snowflake"]);` (check-cad-preload.mjs:313) with a comment:
"EXCLUDED until the empty-Solid mesh fix lands (v-cad MR 1f81340dc ... → `M.union([])` ...). Re-include
it (a one-line edit removing it from SKIP) in the follow-up ... once the fix + re-arch are on trunk."
CONFIRMED the fix is ON TRUNK: `guide/src/cad/index.ts:425` returns `M.union([])` for the `Empty` case
(the composing empty), and `git merge-base --is-ancestor 1f81340dc trunk` = YES. So the precondition the
skip's own comment names has been met — the skip should be removed so the mesh-check covers the snowflake
again (else a blank-snowflake regression slips silently, which is exactly what this gate guards). Real
test-coverage gap, self-documented as the intended follow-up.

## Owner
`guide/scripts/check-cad-preload.mjs` + the /cad snowflake showcase + the empty-Solid fix = v-cad
(area=guide). One-line SKIP removal + confirm the snowflake now meshes >0 verts under the gate.
