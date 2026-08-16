# drm1 — drum sequencer over 16th-note steps (2026-08-16, tick 1595)

(step, hits) state with a let-free 3-voice maskof callee (kick %4, snare
%8==4, hat %seed-div) summed as a bitmask: `hit` answers the mask through a
match binder over the CALL (fence-safe, 2 consumers — guard + answer... note
the binder feeds the silence guard AND the answer, tnk-axis boundary at
exactly 2), advancing the step ring (mod 16); `cnt` counts non-silent steps.

Hat divisions 2 vs 1: the halved rate turns every ODD row silent (0s) while
kick/snare rows COINCIDE exactly (5, 4, 7 at the same positions) — voice-
coincidence anchors with silence interleave, counts 3 vs 6.

PASS ×3. **Pool (13th trio seed).**
