# PR#892 review comment — handle_arm_state_ty doc says init(=parts[1]) but code uses .first() (index 0) (v-inference)

Mirrored from GitHub PR#892 review comment (Copilot), id `3672947563`.
File: `implementation/seed/crates/rcdzc/src/effects.rs:3885`. Blame `520142726` "infer: type a handler
arm's STATE binder from the handle seed (fixes Qty inline-arith next-state false CDZ0201)" — v-inference's
HELD-44 Qty fix (the same workstream as the PR#891 HELD-44 note).

## Comment (verbatim)

- (id 3672947563, effects.rs:3885) "Doc comment says the seed is reached via `handle-internal →
  init(=parts[1])`, but the implementation uses `as_form(handle, HANDLE_INTERNAL).and_then(|t|
  t.first())` (i.e. the first tail element / index 0). This mismatch makes the navigation description
  misleading."

### Liaison verification (confirmed on trunk 18b97d4cb)

`handle_arm_state_ty` (effects.rs, fn from `520142726`). Doc line ~3884-3885: "Navigates `binder(=parts[2])
→ arm → arms-list → handle-internal → init(=parts[1])` and types the seed." But the code (a few lines
below): `let init = db … .as_form(handle, HANDLE_INTERNAL).and_then(|t| t.first().copied())?;` — `t.first()`
is index 0 of the `HANDLE_INTERNAL` form's tail, NOT `parts[1]`. So the doc's "`init(=parts[1])`" cite
disagrees with the `.first()` (index 0) the code uses. Either the doc index is wrong (should say the first
tail element / index 0 of the handle-internal form) or the intended navigation differs — reword the doc to
match `.first()`. Comment-only, behavior-neutral (the code is what it is; only the navigation description
is off). Owner should confirm the CODE is correct (`.first()` really is the seed) and fix the DOC, not the
other way around.

Owner: **v-inference** (their `520142726` handler state-binder-from-seed fix in effects.rs — same HELD-44
Qty workstream as the PR#891 note). Doc reword to match `.first()`/index-0.
