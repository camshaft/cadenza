# PR review comments — mirrored from GitHub PR #402 (Copilot inline)

- **PR:** #402 "fleet: twenty-seventh batch (deadlock fix: duvet-check + 2 recovered lost commits, breaker, quantity)" (MERGED)
- **File:** `implementation/seed/crates/rcdzc/src/backend/wasm/select.rs` (box_op_ty @1357, get_op_ty @1418)
- **Reviewer:** Copilot (automated)
- **Comment ids:** 3590925167, 3590925216
- **Links:** https://github.com/camshaft/cadenza/pull/402#discussion_r3590925167 , #discussion_r3590925216

## Comments (verbatim)
> `box_op_ty` returning `Ok(None)` for `Ty::Unit` will let code paths like `Core::Tuple` / `Core::Record` / multi-payload `Core::SumNew` attempt to store a Unit-typed value into the value heap without pushing any payload value (since `Core::Unit` emits nothing). That yields invalid wasm (stack underflow at `arr-set` / `sum-new`). It's safer to keep Unit unboxable here until emit has an explicit `IMM_UNIT`-handle path for heap stores.
>
> `get_op_ty` returning `Ok(None)` for `Ty::Unit` leaves a u32 heap handle (typically `IMM_UNIT`) on the wasm operand stack when a Unit value is projected, but `valtype_of(Unit) = None` so Unit expressions must leave *no* stack value. This can produce invalid wasm due to stack-type mismatch. For Unit projections, the correct behavior is to consume/drop the handle and leave nothing.

## Liaison triage
Two wasm-backend correctness concerns around `Ty::Unit` in heap stores/projections:
- `box_op_ty` Unit → `Ok(None)`: a Unit element inside a `Core::Tuple`/`Core::Record`/multi-payload
  `Core::SumNew` would be stored without pushing a payload value (Core::Unit emits nothing) → stack
  underflow at `arr-set`/`sum-new` (invalid wasm).
- `get_op_ty` Unit → `Ok(None)`: projecting a Unit leaves an IMM_UNIT handle on the operand stack, but
  `valtype_of(Unit)=None` means a Unit expression must leave NO stack value → stack-type mismatch.
This neighbors my pr388 finding (`closure_type_index` declines Unit results) — the backend's Unit
handling across heap boundaries needs a coherent story (either an explicit IMM_UNIT heap path, or keep
Unit unboxable + drop-on-project). Route to `corpus-bugfix` PM to repro (a tuple/record/sum with a Unit
field; a projection of a Unit element) and confirm/fix. Fix on `trunk`. Quotes + links in queue file.
