# review finding: a refutable let/param inside a NESTED inline lambda still skips CDZ0210

**Severity:** low (escaped diagnostic / decline-discipline gap — NOT a miscompile; refutable
bindings are an erroneous+uncommon shape). Follow-up to #1428 (`da33ad549`), which closed the
gap ONE lambda-level deep but not for a lambda nested inside another inline lambda's body.

**Owner:** v-inference (their queued item B: "REFUTABLE let/param inside a LAMBDA body skips
CDZ0210"). Found by reviewer reviewing the `da33ad549` diff.

## Symptom
A refutable binding-pattern `let` LHS (a literal `5`, a nullary ctor, etc.) escapes CDZ0210 when
it sits inside an inline lambda that is ITSELF nested inside another inline lambda's body. A
single-level inline lambda correctly fires (that is what #1428 fixed); a def body correctly
fires; a doubly-nested inline lambda does NOT.

## Reproducers (probed on trunk da33ad549, `cdz check`, debug build)

Single-level inline lambda — FIRES CDZ0210 (the #1428 fix, correct):
```
(module m (def (main) (let ((g (fn (x) (let ((5 y)) y)))) 9)) (export main))
```

Nested inline lambda — ESCAPES (0 diagnostics; the gap):
```
(module m (def (main) (let ((outer (fn (x) (let ((inner (fn (z) (let ((5 y)) y)))) 3)))) 9)) (export main))
```

Also escapes with an immediately-applied nesting:
```
(module m (def (main) ((fn (g) 1) (fn (x) ((fn (h) 2) (fn (z) (let ((5 y)) y)))))) (export main))
```

Def-body baseline — FIRES (correct, unaffected):
```
(module m (def (main) (let ((5 y)) y)) (export main))
```

## Root cause
`inline_lambda_binding_pattern_faults` (infer.rs, `collect_node`'s `Resolved::Lambda` arm) STOPS
recursion at any nested `fn`, on the premise (its own comment) that "a nested lambda is validated
by its own `collect_node` Lambda-arm entry." That premise holds only for a lambda `collect_node`
actually REACHES. But `collect_node`'s Lambda arm descends the body (`collect(db, body, out)`)
ONLY when `def_index_by_body(id).is_some()` (a named def) or the applied-try case — an inline
lambda's body is otherwise never descended, so a lambda nested inside an inline lambda's body is
never visited by `collect_node`, and the shape-only walk's early-stop-at-`fn` means it is never
reached by the new walk either. Net: the binding-position irrefutability check is closed exactly
one inline-lambda level deep.

## Suggested fix direction (for the owner to weigh)
Instead of stopping at a nested `fn`, RECURSE into a nested inline lambda's body from
`inline_lambda_binding_pattern_faults` too (it is shape-only, so it cannot spuriously fault a
generic/uninstantiated nested body — the very property that makes it safe at the top level makes
it safe at every level). Keep the def-body / applied-try `collect` gating exactly as is; only the
shape-only binding-pattern walk needs to descend nested inline lambdas. `dedup_faults` already
collapses any overlap. Add a nested-lambda witness to the #1428 test.
