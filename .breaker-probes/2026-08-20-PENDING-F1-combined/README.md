# PENDING-F1-combined — hold until v-effects F1 fix lands, then cut ONE combined 14c batch

v-effects handed me 2 verified pins (pyu8t1 + pyu8r1) to append as the single 14c EOF
writer, avoiding their fix+corpus MR's EOF-collision with my batches. BOTH still DECLINE on
trunk 20323bf6d (F1 fix NOT landed). PLAN: the moment F1 lands, rebuild cdz on the fix
commit, re-verify ALL FOUR fold to the stated oracles x3 + opt-sweep, append in one EOF
write with pass-rows x3, gate + roundtrip, send. DO NOT baseline pass while they decline.

## From v-effects (transcribed; ask for verbatim sexp if byte-identical titles wanted):
### pyu8t1 (widened-state axis) — oracle 1170 / 27554 (255+5 wraps to 4)
(effect E (op tick (-> Int64)))
main n = (handle E (UInt8.wrap n)
           ((tick () s (resume (Int64.of s) (UInt8.wrapping-add s (UInt8.wrap 5)))))
           (+ (* 100 (E.tick)) (+ (* 10 (E.tick)) (E.tick))))
call 10 -> 1170 ; call 250 -> 27554

### pyu8r1 (narrow-op-result axis) — oracle 100101 / 255000 (state 256 -> UInt8.wrap answer 0)
(effect E (op get (-> UInt8)))
main n = (handle E n
           ((get () s (resume (UInt8.wrap s) (+ s 1))))
           (+ (* 1000 (Int64.of (E.get))) (Int64.of (E.get))))
call 100 -> 100101 ; call 255 -> 255000

## My auto-flip cases (already banked, oracle at ruled-correct, flip on same fix):
- pyu8w1  .breaker-probes/2026-08-19-uint8-wrap-state/       (251000005 / 250255004)
- pyu8a1  .breaker-probes/2026-08-20-narrowint-answer/       (276453 / 275352)
