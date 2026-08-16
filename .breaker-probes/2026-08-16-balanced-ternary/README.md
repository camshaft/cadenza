# btr ladder — balanced-ternary digitizer (2026-08-16, tick 1602)

Attack: signed-trit peel with a THREE-branch nested-if arm where every branch
rebuilds the full 3-tuple state (v', acc±w, w*3). First balanced-ternary theme.
The ten seed transits NEGATIVE partial sums before the accumulator reconstructs
the original value at the final check.

## Ladder
- **btr1** (5 steps, inline `(% v 3)` in both conditions): DECLINES —
  "emit-walk scratch-locals budget exceeded" (clean decline, the 4209bd054
  budget; error kind = count-limit → F24 class, not width-alias).
- **btr2** (5 steps, remainder HOISTED into an arm-local binder, 2 consumers):
  STILL DECLINES with the same budget error. ⚠️ NEW FENCE DATAPOINT: the
  binder-hoist rescue (tnk lesson: ≤2 consumers safe) does NOT rescue this
  shape — the scratch-slot pressure is minted by the per-branch 3-tuple state
  REBUILDS (each `(tuple (/ ... 3) (± acc w) (* w 3))`), not by the condition
  compound. Workaround guidance must not promise binder-hoisting cures
  3-branch × 5-dispatch × 3-tuple arms.
- **btr3** (4 steps, binder form): PASSES ×3 wasm + rust + rust-async.
  Fence sits between 4 and 5 dispatches for this arm shape.

## Model (hand-verified python, banked in transcript)
- 5-step: n=10 rows [9,9,0,9,1] acc 50 → 9909150; n=0 rows [1,1,1,1,0] acc 40 → 1111040
- 4-step (btr3): n=10 → 990869 (acc still NEGATIVE-transited mid-value 8*... acc -31+... = 69 offset packing); n=0 → 111140

Trunk 5c8e8e9a3. All three backends agree on btr3. btr1/btr2 held from corpus
(await (b) sharing-aware emit joint pass-pin).

Ops note: `cdz compile` takes the BARE (do ...) program, not the (case ...)
wrapper — extract before classifying decline errors.
