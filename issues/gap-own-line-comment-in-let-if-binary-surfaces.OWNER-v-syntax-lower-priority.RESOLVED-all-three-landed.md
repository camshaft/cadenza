# gap: own-line `//` comment dropped in let-binding / if-branch / binary-segment positions

**Owner:** v-syntax (self-filed 2026-07-22). LOWER PRIORITY — the common comment surfaces are DONE.

## Status of the `//` comment-preservation workstream
DONE (landed): collection literals (list/tuple/set/record/map — same-line trailing + own-line interior), call arguments (leading + last-trailing), match arms (leading + trailing), sum-type variants (leading + trailing). The reusable pattern is in the vertical log: reader drains leading comment BEFORE the separator into `pending_leading`; shape-guard peels via `strip_field_comments`; printer peels leading-loop (above) + trailing (after). Shared helpers: `strip_field_comments`, `has_nonlast_comment_after`.

## Remaining surfaces (each still DROPS an own-line `//`, fmt-refuses — no corruption)
1. **let-binding — ✅ DONE (MR `fe92ebcdb` sent 2026-07-22):** let_expr drains + wraps the `(binder value)` pair; `is_let_shape` peels via `strip_field_comments`; `print_let` peels leading `(comment)` in a loop above the binding (hardbreak forces the break) + trailing after the value. Faithful + idempotent; compiles to wasm.
2. **if-branch — ✅ DONE (MR `1fdd436a8` sent 2026-07-22):** turned out READER-ONLY — if_expr drains the cond/then/else leading slots + wraps `(comment "text" expr)`; the printer already renders a leading `(comment …)` above an expr (no print_if change needed). ⇒ **ALL THREE filed surfaces DONE. The `//` comment-preservation surface is COMPLETE.** This queue file can be resolved.
3. **binary-segment — ✅ DONE (MR `5d384a313` sent 2026-07-22):** was the cheapest, as predicted — `bin_form` gained the leading + last-segment-trailing capture, `print_bin` switched to `bracketed_comment_aware`. (bin-PATTERN comments captured but not rendered → drop-guard-refused, rare, left as-is.)

## Why lower priority / not rushed
Own-line comments inside `let` bindings / `if` branches / `b[]` segments are UNCOMMON in practice (vs. documenting a match arm, type variant, or collection element). Each needs its own shape-guard + printer-layout work with escalating complexity and diminishing marginal value. The current state is SAFE (drop-guard refuses, no corruption). Pick these up opportunistically or if a real consumer hits one. `binary-segment` (item 3) is the cheapest if resuming — closest to the landed bare-value pattern.

Route: v-syntax self, no cross-vertical dep.
