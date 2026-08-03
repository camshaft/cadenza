# PR #1721 review comment — rcdzc/src/backend/wasm/select.rs (v-effects) — MERGED, fix-forward

https://github.com/camshaft/cadenza/pull/1721 (MERGED).

## Core::Seq stmt-drop uses `type_of == Ty::Unit` without strip_nominal → nominal-Unit → Lir::Drop on empty stack → invalid wasm (Copilot, select.rs:11547) — correctness [VERIFIED]
> `Core::Seq` drops a non-`Unit` statement result via `matches!(type_of(..), Ty::Unit)`, but nominal-Unit
> types (`Ty::Nominal { inner: Unit }`) have NO runtime rep (`valtype_of` reads through nominals). The
> emitted statement leaves nothing on the wasm stack, yet we still push `Lir::Drop` → stack underflow /
> invalid module. Use `strip_nominal()` (or `valtype_of(..).is_some()`).

VERIFIED against trunk (select.rs:11547): the host-reaching Seq stmt path emits then does `if
!matches!(crate::infer::type_of(db, *s), Ty::Unit) { out.push(Lir::Drop) }` — WITHOUT strip_nominal. A
nominal-Unit statement result (`Nominal{inner:Unit}`, e.g. a newtype over Unit) isn't `Ty::Unit`, so it
takes the drop branch — but it has no wasm value on the stack → `Lir::Drop` underflows → invalid module.
CONFIRMING the pattern: the SAME file at select.rs:1108 already uses
`!matches!(type_of(db, field).strip_nominal(), Ty::Unit)` for the analogous field check — so this site is
inconsistent. Fix: add `.strip_nominal()` (matching :1108). MED (needs a nominal-Unit host-call result
discarded in a Seq — narrow, but produces invalid wasm when hit). Fix-forward.
