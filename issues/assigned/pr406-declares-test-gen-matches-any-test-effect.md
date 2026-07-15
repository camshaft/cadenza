# PR review comment — mirrored from GitHub PR #406 (Copilot inline)

- **PR:** #406 "fleet: thirty-first batch (slack-bridge CI gate, generics F1, try-operator, beta.cdz, corpus)" (MERGED)
- **File:** `implementation/seed/crates/rcdzc/src/proptest_gen.rs:189`
- **Reviewer:** Copilot (automated)
- **Comment id:** 3591116791
- **Link:** https://github.com/camshaft/cadenza/pull/406#discussion_r3591116791

## Comment (verbatim)
> `declares_test_gen` currently returns true for *any* `(effect Test ...)` declaration, even when it does not declare a `gen` op. That can suppress synthesis of the `(op gen (-> Unit Int64))` effect and make the generated wrappers call a non-existent `Test.gen` (e.g. the codebase already has tests that declare `(effect Test (op fail ...))`). This should specifically check for an `op gen` inside the `effect Test` form.

## Liaison triage — CONFIRMED against trunk
Confirmed in proptest_gen.rs: `declares_test_gen` iterates items, and for an `(effect …)` form whose
first name is "Test" it `return true` — it does NOT check for an `op gen` inside. But its NAME and use
(guarding synthesis of `build_test_effect` = `(effect Test (op gen (-> Unit Int64)))`) mean it should
detect the `gen` op specifically. So a program that declares a DIFFERENT Test-effect op — e.g.
`(effect Test (op fail …))`, which the reviewer says already exists in the codebase — makes
`declares_test_gen` return true, suppressing the `gen` synthesis, and the generated property wrappers
then call a non-existent `Test.gen`. Real correctness bug in the property-testing codegen. FIX: check
for an `(op gen …)` inside the `(effect Test …)` form, not just the effect name. Property-testing is a
recent corpus/fleet workstream with no dedicated vertical → route to `corpus-bugfix` PM. Fix on `trunk`.
Quote + link in queue file.
