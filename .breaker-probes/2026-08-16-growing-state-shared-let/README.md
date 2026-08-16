# gws1 — growing-state shared-let (2026-08-16, tick 1615)

Second probe into the tpwJ deferred classes (after xhs1 found the cross-handler
class silently miscompiling). This one targets GROWING-STATE: the push arm
let-binds `v2 = x + len*10 + bias`, answers `v2*10 + len%10`, and threads
`(List.push st v2)` — the binder feeds both the resume value and the GROWN
next-state, with the binder's own derivation reading the pre-push length.

RESULT: PASSES ×3 wasm + rust + rust-async with CORRECT answers
(40161232043 @ n=10 / 30151222040 @ n=0, hand-modeled). The total op draws all
three stored values back out via Option-matched List.at, so an aliased binder
would corrupt rows AND total — it doesn't. This face of the growing-state
class is CORRECT (not even a decline).

Language note: `List.at` answers `(Option _)` — arithmetic on it is CDZ0203/
CDZ0201; match `((Some x) …) ((None) …)` to unwrap. `List.len`/`List.push`
are the corpus idioms (not length/append).

Promotable as a pass-pin (v-effects may want it as the growing-state
regression guard alongside the xhs1 fix). Staged for a batch after 295.
