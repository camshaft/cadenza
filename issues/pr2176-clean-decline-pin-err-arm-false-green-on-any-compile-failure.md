# PR #2176 review — rcdzc/src/tests.rs (v-effects) — OPEN — test-precision [VERIFIED, LOW-MED]

https://github.com/camshaft/cadenza/pull/2176 (pin performing-closure×indirect-call is a clean decline,
never a miscompile — breaker cc-family). Copilot 1 inline on the pin's error arm.

## the `Err(_) => {}` arm makes the pin false-green on ANY compilation failure (a coded rejection / real regression / unrelated break), not just the intended CLEAN (uncoded) decline (Copilot, tests.rs:68193) — test-precision [VERIFIED, LOW-MED]
> `Err(_) => {}` makes the new test false-green on *any* compilation failure (including coded rejections /
> real regressions unrelated to this decline). Since the intent is to pin a *clean decline* for the
> indirect-call case, the error arm should assert the decline is uncoded (and ideally keep the failure
> message).

VERIFIED in the #2176 diff: the test matches `compile_component(...)` (diff:46) with:
  - `Err(_) => {}` (diff:48) — comment "Clean decline (the current, expected behavior) — fine, so long as
    it's a decline, not a crash."
  - `Ok(bytes) => { ... assert_eq!(v, EXPECT, "...must equal the direct value 204, never miscompile") }`
    (diff:51-56) — the Ok arm is tight.
So the Ok side is well-guarded (if it ever folds, value MUST match the direct form — good). But the Err
side swallows EVERY error: a bare `Err(_) => {}`. The pin's stated intent is a CLEAN decline (an uncoded
"not yet supported" decline), yet a future CODED rejection (a CDZ diagnostic firing where it shouldn't, a
real regression in this path, or an unrelated compile break in the fixture) would ALSO hit `Err(_) => {}`
and pass silently — false-green. That defeats the pin: a breaker corpus pin exists to CATCH the miscompile,
and a too-loose error arm means a regression that turns the clean decline into a coded error (or crash
surfaced as Err) goes unnoticed. LOW-MED/test-precision (breaker pin — no shipped bug, but the guard is
weaker than it reads). Fix per Copilot: assert the `Err` is the EXPECTED uncoded decline — match the
decline shape (uncoded / no CDZ code) rather than `_`, and keep the failure message in the assert so a
coded rejection fails loudly. (Same test-precision class as prior "assert the specific outcome, not just
that it's non-Ok" findings.) v-effects owns rcdzc. PR OPEN → foldable pre-merge. (The Ok-arm value-equality
guard is the important half and it's correct; this tightens the Err half.)
