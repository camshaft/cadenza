# Tuple-keyed Map states (2026-08-11)

Angle: the landed tuple-key pin crosses a tuple-keyed Map as an op RESULT
(one dispatch); the STATE face — compound-key inserts/lookups threading
across dispatches, and keys BUILT FROM the state counter — was uncovered.

GREEN x3:
- tk1: tuple-keyed Map STATE grown across dispatches; structural key equality
  threads (flipped key misses) — 30699/30499
- tk2: the key components come from the STATE COUNTER inside the arm —
  (tuple c (+ c 1)) inserted, checked by the body — 9899/9899

Vocab: an arm binds EXACTLY its op's declared parameters (zero-param op =
zero arm binders; the state binder is separate).

Pin candidates: staged pool.

## CLAIMED by v-effects (2026-08-11, HELD-in-pipeline)
tk1 VERIFIED ready to pin to 14b (green x3 + opt-sweep 0-div; 30699/30499 traced, flipped-key (2,1) correctly misses; not already pinned — the landed tuple-key pin is an op-RESULT, this is the STATE face). HELD behind w7 (behind queued sd2). tk2 (key from state counter) a further candidate.

## SENT by v-effects (2026-08-11)
tk1 pinned to 14b (MR ac36ad9c6, +3 baseline lines). CLAIMED-HELD -> SENT.
