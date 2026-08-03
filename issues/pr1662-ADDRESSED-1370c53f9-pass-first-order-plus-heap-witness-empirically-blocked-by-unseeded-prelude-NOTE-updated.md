# PR #1662 review comments — rcdzc/src/opt.rs (v-core-opt) — MERGED, fix-forward

https://github.com/camshaft/cadenza/pull/1662 (MERGED — the fix for my #1652 CSE-guard-witness finding).
Copilot re-engaged with two follow-ups on the fix itself.

## 1. Capturing witness lowers body BEFORE running the pass — less faithful (Copilot, opt.rs:934) — test-precision
> The comment describes the hazard as "pass-time `core_of` is the first demand before lift/capture
> context exists", but the test lowers the body with `lower::core_of` BEFORE running `GlobalCsePass`.
> That can pre-populate lift/capture context and makes the test less faithful. Run the pass first, then
> lower.

The witness pre-lowers, which can seed the very lift/capture context the hazard says shouldn't exist yet
— so the test may not faithfully reproduce "pass-time core_of is the first demand". Run GlobalCsePass
BEFORE any lower::core_of so the pass doesn't benefit from pre-lowered state. LOW-MED/test-precision.

## 2. Heap-type guard-A witness IS constructible — Copilot refutes the "impractical" NOTE (Copilot, opt.rs:959) — test-coverage [VERIFIED plausible]
> The NOTE says the heap-type exclusion (guard A) can't be unit-tested and leaves it to e2e. But you can
> construct an eligibility-clean witness where the repeated subexpression itself is heap-typed — e.g. two
> identical `(. r b)` reads of a `Bytes` field as the two operands of `Bytes.concat`. That passes
> `body_is_pure_scalar` but must still yield NO overrides because guard A excludes heap-typed candidates.
> Replace the NOTE with a concrete unit test.

VERIFIED plausible against opt.rs: `body_is_pure_scalar` (opt.rs:269) ADMITS `Resolved::Member`/`Proj`
(the `… | Member { .. } | … => true` arm at :292-293). So a body of two `(. r b)` Bytes-field reads IS
pure-scalar-eligible, while the repeated `(. r b)` is a heap-typed CANDIDATE — exactly the guard-A input
you said (in your #1652 fix note) was impractical to isolate. Copilot's construction directly refutes that:
the Member read passes eligibility, so guard A (not the eligibility gate) is what must suppress it. This
REOPENS the guard-A-unit-test question with a concrete witness — worth re-evaluating whether it works
(you own that coverage call). If it does, replace the NOTE with the real test; if there's a reason it
still can't attribute cleanly, the NOTE should say WHY vs this specific construction. MED/test-coverage.
