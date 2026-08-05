# PR #2243 review — rcdzc/src/tests.rs (v-effects) — OPEN — 2 test-precision [VERIFIED, LOW-MED]

https://github.com/camshaft/cadenza/pull/2243 (pin multi-shot arm folds flat but declines inside
recursion, never miscompiles; breaker ms-family). Copilot 2 inline — same false-green class as my
#2176/#2195/#2216 breaker-pin findings.

## the flat-fold case uses `run_linked` + `if let Some(v)` → the value assertion is SILENTLY SKIPPED if the runtime wasm is absent, weakening the guard; use `run_returns::<i64>` so the check always runs (Copilot, tests.rs:68174) — test-precision [VERIFIED, LOW-MED]
> This test uses `run_linked` (and `if let Some`) even though the program returns an `Int64`. If the
> runtime wasm is absent, the assertion is silently skipped, weakening the guard. Prefer `run_returns::<i64>`
> here so the value check always runs and is typed as an integer.
VERIFIED in the diff: the flat multi-shot case does `if let Some(v) = run_linked(&flat_bytes, "main") {
assert_eq!(v, "25", …) }` (diff:27-30). The `if let Some` means: if `run_linked` returns `None` (runtime
wasm absent), the `assert_eq!(v, "25")` NEVER RUNS → the pin passes without checking the cross-product
value (2*2+2*3+3*2+3*3=25). LOW-MED/test-precision (a breaker pin that silently skips its value check is a
weak guard). Fix per Copilot: `run_returns::<i64>(…)` so the value assertion always runs + is typed as the
Int64 the program returns.

## the recursive-branch case accepts `Err(_)` (masks unrelated CODED rejections) and, if it compiles, only checks "no crash" (ignores the returned value) → doesn't actually guard the miscompile (Copilot, tests.rs:68190) — test-precision [VERIFIED, LOW-MED]
> The recursive branch currently accepts `Err(_)` (which can mask unrelated coded rejections) and, if it
> compiles, only checks for "no crash" by ignoring the returned value. To actually guard against
> miscompiles, assert that an error is an *uncoded decline*, and if it ever compiles, assert the correct
> result (25 for `loop 2`).
VERIFIED in the diff: the recursive case has `Err(_) => {}` (diff:42) and `let _ = run_linked(&bytes,
"main");` (diff:46) — the Ok path DISCARDS the value ("no crash" only). So a future CODED rejection (a
regression, a CDZ diagnostic firing wrongly) passes via `Err(_)`, AND if it ever folds, a WRONG value goes
unchecked. This is the SAME false-green shape as my #2176 (cc-family `Err(_)`) — recurring on the ms-family
pin. LOW-MED. Fix per Copilot: match the `Err` to an UNCODED decline (not `_`), and on the Ok path assert
the correct result (25 for `loop 2`) rather than discarding it — so the pin guards the miscompile it exists
for. v-effects owns rcdzc + the breaker pins. PR OPEN → both foldable. (These pin-precision findings recur
across the breaker families — cc #2176, ob #2195, this ms — worth a standing "pin value-asserts + uncoded-
decline-matches, never Err(_)/skip" convention.)
