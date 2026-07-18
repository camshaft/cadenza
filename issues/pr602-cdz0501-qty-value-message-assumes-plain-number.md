# pr602 — CDZ0501 `Qty.value` message says "a plain number" but fires for ANY non-quantity operand

Mirrored from GitHub PR #602 review comment (Copilot), id 3609102456.
PR: https://github.com/camshaft/cadenza/pull/602 (8-MR publish batch)
Location: `implementation/seed/crates/rcdzc/src/infer.rs:7883`

## Reviewer comment (verbatim)
> The CDZ0501 message for `Qty.value` assumes the operand is "a plain number", but this check triggers
> for any well-typed non-quantity operand (e.g. Bool/String/Record). That makes the diagnostic
> misleading in non-`Unit.in` cases. Reword it to say "not a quantity" generally, and mention
> `Unit.in`/`as` unwrapping only as a common cause/repair.

## VERIFIED (git show trunk)
The `Qty.value`-operand-not-a-quantity reject (infer.rs ~7877) builds:
`"`Qty.value` recovers a quantity's number, but this operand is {render_with_article()} — a plain
number, not a quantity (an `as`/`in` conversion already UNWRAPS to a bare number ... drop it)"`.
`render_with_article()` DOES name the real type, but the trailing "— a plain number, not a quantity"
clause hardcodes the assumption that the operand is a number. For a `Bool`/`String`/`Record` operand
the message reads "…this operand is a Bool — a plain number, not a quantity" — self-contradictory.
Real diagnostic-quality nit. Fix (per Copilot): say "…is {ty}, which is not a quantity" generally, and
demote the `as`/`in`-already-unwrapped note to a "common cause / repair" hint. Minor, no behavior change.

## Owner
`rcdzc/src/infer.rs` CDZ0501 diagnostic wording = v-inference (owns infer + CDZ diagnostic codes).
