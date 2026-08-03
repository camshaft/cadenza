# PR #1245 review comments — rcdzc/src/lower.rs (v-patterns)

Mirrored from https://github.com/camshaft/cadenza/pull/1245 (PR: "cand: v-patterns — f90255fd2").
Both bots flag the `(do …)` special-case in `collect_binding_uses` at lower.rs:13036.

## 1. Early return may skip tail-expr resolution for `do` blocks (amazon-q, lower.rs:13036) — correctness, VERIFY
> Logic Error: The early return at line 13035 skips the `resolved_of` resolution for `do` blocks, but
> the function should still process the tail expression after handling statements. The `return`
> prevents the tail from being analyzed through `resolved_of`, which could miss uses in the final
> expression.
> [suggests removing the early return so execution continues to `match resolved_of(db, node)`]

⚠ VERIFY against intent: if the `do` tail's uses are genuinely collected elsewhere this may be a
false alarm, but if the early return means the tail expression's refs are never analyzed, that's a
real miss. Confirm whether the tail is handled by the statement loop or needs the `resolved_of` pass.

## 2. `do` special-case ignores `proj_operand`, may spuriously mark `escapes_whole` (Copilot, lower.rs:13036) — correctness
> `collect_binding_uses`'s special-case for `(do …)` currently walks every form with
> `proj_operand=false`. If the whole `do` appears in a projection/member operand position (i.e. the
> caller passed `proj_operand=true`), the tail form should be collected with `proj_operand=true` so a
> bare `Ref` in the tail is not incorrectly recorded as a whole-value escape. This can spuriously
> mark bindings as `escapes_whole` and change keep/copy-propagation decisions.

This one is concrete: the `do` tail inherits the caller's `proj_operand`, but the special-case hard-codes
`false`, so a `do` in operand position mis-records its tail `Ref` as a whole-value escape → wrong
keep/copy-propagation. Thread `proj_operand` through to the tail form.

(Both comments are on the same line/construct — worth addressing together: the tail needs both the
`resolved_of` analysis AND the caller's `proj_operand`.)
