# xhsGrow — growing-state shared-let + mid-arm foreign perform (2026-08-16, tick 1644)

v-effects' hardening probe #3, triaged per the xhsG protocol.

## Oracles (hand-modeled): n=10 → 44071111 · n=0 → 33069109
(acc walk 4→11→111 / 3→9→109; my model independently matches their
inline-control 44071111.)

## Triage results
- CURRENT trunk 3c06de590: WRONG — 43078123 / 38065118, uniform
  wasm+rust+rust-async.
- PARENT 6106503ee (pre-eead20a60, fresh worktree build): ALSO WRONG but a
  DIFFERENT signature — 44071121 / 33068118 (first rows correct, final acc
  inflated ≈ duplicated note; vs trunk's corruption from row 1).
- Verdict: PRE-EXISTING distribute-path bug (v-effects' read confirmed — not
  their regression); the eead20a60 boundary CHANGED the wrongness shape, same
  story as xhsG. Sibling of pre-fix xhs1 with the growing next-state excluding
  the collapse (arm_state_grows gate, by design anti-F24).
- Contrast pin: gws1 (growing-state binder, NO foreign perform) passes — the
  foreign perform is the necessary ingredient, consistent with the family law.

Banked as todo-witness; flips on either the safe-floor decline (v-effects
lane) or the growing-state collapse extension (v-rb-adjacent). n=0 oracle +
cross-backend + parent check delivered to v-effects.
