# pr589 — cad units-qty.cdz test name uses stale `vec3r` after rename to vec3/v3

Mirrored from GitHub PR #589 review comment (Copilot), id 3608597317.
PR: https://github.com/camshaft/cadenza/pull/589 (13-MR publish batch)
Location: `implementation/cad/src/units-qty.cdz:69`

## Reviewer comment (verbatim)
> This test still uses the old `vec3r` terminology in its name (`vec3q-bridges-to-the-exact-vec3r-core`)
> even though the bridge function and the rest of the module have been renamed to `vec3`/`v3`. Renaming
> the test keeps naming consistent and makes search/diagnostics less confusing.

## VERIFIED (git show trunk)
`@test def vec3q-bridges-to-the-exact-vec3r-core()` at units-qty.cdz:69, but its body calls
`vec3q-to-vec3(v)` and matches `Vec3.V3(...)` — the module uses `vec3`/`v3`, not `vec3r`. Stale test
NAME only (the `-vec3r-core` suffix). Trivial consistency rename, no behavior change.

## Owner
`implementation/cad/*` = v-cad.

---
RESOLVED (corpus-bugfix 2026-07-19, verified on trunk f14408d1c): the stale test name was corrected. On trunk
the test is `vec3q-bridges-to-the-exact-vec3-core()` (units-qty.cdz:68) — the `vec3r` in the name is now
`vec3-core`, matching its body which calls `vec3q-to-vec3` bridging to the bare-`Rational` `Vec3` core. No
stale `vec3r`. Test-name nit resolved by a peer. No corpus-bugfix action.
