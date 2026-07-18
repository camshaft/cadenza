# Misleading "runtime record" decline: Record.project over a MULTI-USE let-bound (all-const) record

**Reporter:** breaker (2026-07-18). **Severity:** misleading diagnostic + capability-cliff. NOT a miscompile — a decline with a wrong message. Both backends agree (rust also `declined`).

## Finding
`Record.project` (and the row-op family) over a let-bound record USED MORE THAN ONCE declines "a record row operation over a **runtime** record is not yet built" — even when every value is a COMPILE-TIME CONSTANT. The word "runtime" is wrong; the real trigger is that a shared `let` binding isn't substituted to a literal `Core::Record` at the projection site, so `core_of(db, record)` in `lower_record_project` (lower.rs:19992) sees a `Core::Var`, not a `Core::Record`, and falls to the non-constant decline arm.

## Minimal pair (both all-const, no runtime value) — VERIFIED on trunk 51088e875
```
SINGLE use  compiles+runs:  (let ((r (record (f 5) (g 8)))) (. (Record.project r (f)) f))                                   -> 5   (check rc=0)
DOUBLE use  declines:       (let ((r (record (f 5) (g 8)))) (+ (. (Record.project r (f)) f) (. (Record.project r (g)) g)))  -> "runtime record" decline
```
Same split for a let-bound record projected once then accessed directly (`(. r g)` after a project also declines). Inline (non-let) double-project compiles fine (the fold sees the literal each time).

## Two sub-issues
1. **DIAGNOSTIC [actionable]:** the message says "runtime record" for an all-const record — should say the row op needs a record literal at the site / doesn't yet see through a shared binding (mirror the "requires a record, found <T>" wording from the recent `a_record_row_op_over_a_non_record_names_the_kind` improvement, tests.rs:23578 sibling).
2. **CAPABILITY [lower priority]:** the const-fold for row ops doesn't see through a multi-use `let`. A shared binding of a constant record could still fold. Sanctioned "not yet built" is fine, but single-use-folds / multi-use-declines is a surprising cliff.

## Routing
ROUTED to v-inference (corpus-bugfix 2026-07-18): lower.rs const-fold + diagnostic-quality territory (the sibling diagnostic improvement was theirs). Bounce to v-patterns if record-row-ops are theirs. No corpus repro pinned yet (it's a decline); breaker will add a decline-pin once the message is fixed. Not spawning a fixer (diagnostic/capability in lower.rs, owner territory).
