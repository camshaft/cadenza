# PR #1903 review comments — rcdzc/src/{unify,tests}.rs (v-inference) — MERGED, fix-forward

https://github.com/camshaft/cadenza/pull/1903 (MERGED).

## 1. APPLY_VAR_CHAIN_LIMIT (10k) vs native recursion → stack overflow before the guard on small-stack targets (Copilot, unify.rs:58) — robustness/crash-safety [VERIFIED]
> `APPLY_VAR_CHAIN_LIMIT` is 10,000, but `Subst::apply` uses native recursion; on targets not on an
> enlarged worker stack (notably wasm32 per host.rs), the stack could overflow BEFORE this guard is
> reached. Tie the limit to `crate::db::DESCENT_DEPTH_LIMIT` (+ host stack sizing) so the guard fires
> before stack exhaustion.
VERIFIED: `apply_depth` (unify.rs:89) is natively recursive (`Some(t) => self.apply_depth(t, chain+1)`
:111, + recursion through compound arms), with the cycle-guard at `chain >= APPLY_VAR_CHAIN_LIMIT` = 10,000
(:58/:106). But `DESCENT_DEPTH_LIMIT = 1024` (db.rs:704) is the conservative stack-sizing bound (its doc
notes walks nest on top of core_of's descent). So a ~10k-deep var-chain recurses ~10k native frames —
~10x past the 1024 policy — risking stack overflow on wasm32/small-stack BEFORE the 10k guard fires. Fix:
set APPLY_VAR_CHAIN_LIMIT to (or below) the stack-sizing policy (tie to DESCENT_DEPTH_LIMIT), so the guard
is guaranteed to fire before exhaustion. LOW-MED (needs a ~10k var-chain — a cycle-adjacent unify path
could reach it; a stack overflow is a hard crash, not a clean CDZ reject). Fix-forward.

## 2. Test comment says code/message not pinned, but the test asserts code == CDZ0203 exactly (Copilot, tests.rs:24317) — doc/test
> The comment says the exact code/message is not the pinned property, but the test immediately asserts the
> code is exactly CDZ0203.
Contradiction — reword the comment to match (the code IS pinned as CDZ0203), or if the intent is to NOT
pin the code, drop the exact assert. LOW/test-precision.
