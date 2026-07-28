# PR#766 review comment — infer.rs provisional-operand-defer comment says "both-provisional" but condition is EITHER-Any

Mirrored from GitHub PR review comment (Copilot), id `3627400102`.
PR: https://github.com/camshaft/cadenza/pull/766 (merged; fix still belongs on trunk)
Location: `implementation/seed/crates/rcdzc/src/infer.rs:4920`

## Comment (verbatim)

> The comment says this defer "only catches the both-provisional case", but the condition triggers
> when *either* operand is `Ty::Any` (`matches!(a, Ty::Any) || matches!(b, Ty::Any)`). This mismatch
> makes the intent harder to reason about when debugging type inference.
>
> Consider updating the comment to describe the actual condition (either-side provisional operand)
> rather than "both-provisional".

## Liaison verification (CONFIRMED on trunk)

The PROVISIONAL-OPERAND DEFER block (infer.rs ~4907-4922) ends its rationale:
> "(A single anchoring `BigInt`/`Float` literal operand — the `(+ 1N (f …))` shape — already grounds
> via the arms below, so this only catches the **both-provisional** case the arms cannot yet classify.)"

But the guard is:
```rust
if !db.solving_schemes.is_empty()
    && (matches!(a, Ty::Any) || matches!(b, Ty::Any))   // EITHER operand Any, not both
    && matches!(prim, Add | Sub | Mul | Div | Rem)
```
It fires when EITHER operand is `Any`. The parenthetical's claim that a single anchoring literal
"already grounds via the arms below" is misleading: those numeric arms are BELOW this defer, so a
`(+ 1N <self-call-returning-Any>)` (one concrete BigInt operand, one `Any`) ALSO satisfies
`either-Any` and defers HERE first — it does not reach the arms. So the effective condition is
"either operand is provisional (`Any`) under an in-flight scheme solve", not "both-provisional".

Doc-only mismatch (the CODE behavior is presumably intended — deferring on either-Any is the safe
choice while a self-call result is unresolved; committing a half-anchored arith result could still
freeze the wrong width). Fix: reword the closing clause to "so this catches the either-side-provisional
case the arms cannot yet classify" (and drop/così-correct the "already grounds via the arms below"
implication, since the defer precedes the arms).

Owner: v-inference (owns rcdzc infer/unify/resolve). Routed as a note. (If the INTENT was actually
both-Any-only, that's a code change, not a doc fix — flag that to v-inference to decide; but the
either-Any behavior reads as deliberate.)
