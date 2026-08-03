# PR #1733 review comment — rcdzc/src/backend/wasm/select.rs (v-effects) — MERGED, fix-forward

https://github.com/camshaft/cadenza/pull/1733 (MERGED — the strip_nominal fix for my #1721 Seq-drop
finding). STRONGER FIX NEEDED: strip_nominal==Unit is necessary-but-insufficient.

## Seq stmt-drop uses `strip_nominal == Unit`, but `valtype_of` is None for MORE than Unit (Char/Type/Var/Any) → still underflows (Copilot, select.rs:11553) — correctness [VERIFIED]
> The Seq stmt-drop decides whether to emit `Lir::Drop` by comparing the solved type to `Unit` (after
> strip_nominal). That's NOT the same as "leaves a machine value": `valtype_of` returns `None` for other
> no-representation types too (Char, Type, Var, Any in backend/wasm/lir.rs), so this can STILL emit a
> `Drop` on an empty stack and underflow. Use `valtype_of(..).is_some()` as the predicate.

VERIFIED: the merged code (select.rs:11553, the #1721 strip_nominal fix) is `if !matches!(type_of(db, *s)
.strip_nominal(), Ty::Unit) { out.push(Lir::Drop) }`. But `valtype_of` (lir.rs:363) returns `None` for
`Ty::Unit` AND `Ty::Char` (confirmed: lir.rs `Ty::Char => None`), plus Type/Var/Any. A Seq statement of
type Char (no runtime slot) is NOT Ty::Unit → still takes the drop branch → `Lir::Drop` on empty stack →
underflow. My #1721 strip_nominal note was necessary-but-INSUFFICIENT; Copilot's `valtype_of(..).is_some()`
is the semantically-correct predicate ("did this statement leave a machine value to drop?"). MED — replace
the `strip_nominal == Unit` check with `crate::backend::wasm::lir::valtype_of(&type_of(db,*s)).is_some()`.
(Note: the sibling sites at ~1108/~2486 that #1721 mirrored likely have the same latent gap — worth a sweep
while here.) Fix-forward.
