# Gap: `expect` must trap on overflow-add and on the absent optional case (2 standing gate TODOs)

**File:** `spec/semantics/02-binding-and-control.sexp` — cases "expect on an overflowing checked add
traps" and "expect traps on the absent case of a runtime optional" grade TODO.
**Confirmed:** both are standing todos on current trunk.

`expect` on a computation whose value is observed must propagate that value's trap (an overflowing
checked add must trap), and `expect` on the absent case of a runtime `Option` must trap with its
message. Make both cases pass; confirm against the spec text. The cases already exist as todos.

Area: rcdzc `expect` lowering. Coordinate with whoever owns the `expect`/optional surface.

<!-- DEFERRED 2026-07-15 (operator): needs spec §299 message-carrying traps (LARGE — wasm unreachable carries no text; gate classifies 4 trap KINDS only). The 2 cases are correctly TODO; do NOT force a pass by kind-matching. Revisit when a real program needs message-carrying traps. Not a bug. -->
