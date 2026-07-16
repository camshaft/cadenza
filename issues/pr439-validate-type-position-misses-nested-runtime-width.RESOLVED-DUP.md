# PR review comment — mirrored from GitHub PR #439 (Copilot inline)

- **PR:** #439 "fleet: batch 67+68 (…, sum-payload retain guard, …)" (OPEN at triage; file on trunk)
- **File:** `implementation/seed/crates/rcdzc/src/compile.rs:794` (`validate_type_position`)
- **Reviewer:** Copilot (automated)
- **Comment id:** 3592610268
- **Link:** https://github.com/camshaft/cadenza/pull/439#discussion_r3592610268

## Comment (verbatim)
> `validate_type_position` only checks `is_runtime_width_type(db, pos)` at the top-level node. But in this function `pos` is often a whole payload like `(List (Int n))` (see `push_payload_type_positions`), and `typeval_of` will succeed after `reduce_ctor` clamps the runtime width to a sentinel. That means a runtime width nested inside a compound payload (e.g. `(type T (Mk (List (Int n))))`) can still slip past this validation even though the comment here says it's rejected.
> Consider descending the type-expression subtree to find and reject the offending `(Int n)`/`(Float n)` node (mirroring `infer.rs`'s nested runtime-width check) and anchoring the diagnostic at that node.

## Liaison triage — CONFIRMED against trunk — CHECKER SOUNDNESS HOLE (nested)
Confirmed in compile.rs: `validate_type_position` calls `is_runtime_width_type(db, pos)` only on the
TOP-LEVEL `pos`. For a compound payload like `(List (Int n))` that top check is false (pos is a List),
so a runtime width nested INSIDE slips through — and `typeval_of(db, pos)` then returns Some because
`reduce_ctor` clamps the nested runtime width to a sentinel. So `(type T (Mk (List (Int n))))` with a
runtime `n` passes validation despite the comment claiming rejection. This is the NESTED counterpart of
my pr425 float-width `NotConst` hole (there: is_ill_formed_float_width returns false for a runtime
width; here: the nesting isn't descended at the ctor-payload validation site). FIX (as reviewer):
descend the type-expr subtree to find + reject the offending `(Int n)`/`(Float n)` node (mirror
infer.rs's nested runtime-width check), anchoring the diagnostic there. Checker soundness → corpus-bugfix
PM (pairs with pr425). Fix on `trunk`. Quote + link in queue file.
