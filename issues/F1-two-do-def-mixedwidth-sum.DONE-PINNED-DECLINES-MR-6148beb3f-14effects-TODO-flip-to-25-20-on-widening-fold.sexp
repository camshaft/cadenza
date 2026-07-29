;; HELD PIN (corpus-bugfix, 2026-07-28) — do NOT land until v-effects' safe-floor decline (5cf911aeb)
;; lands. Origin: breaker FINDING (issue 000000017688 F1). Two do-def-bound performs summed under a
;; handler where the fold constructs a mixed-width (+ state:i64  x:UInt8) used to emit an INVALID wasm
;; module (func[0], 'expected i64 found i32'); rust computed 25. v-effects FIXED it to the SAFE FLOOR
;; (5cf911aeb): reduce_handle now DECLINES cleanly ('not yet reducible' todo) rather than emit invalid
;; wasm — it does NOT yet compute 25/20 (that needs a widening-coercion fold widening the narrow
;; operand to the i64 state carrier, a LATER increment). So the CORRECT pin is (declines), NOT 25/20.
;; ⇒ EXPECT (declines) on all backends once 5cf911aeb lands (was invalid-module on wasm / 25 on rust —
;; the invalid module was the bug; the clean decline is the safe floor). When the widening-fold
;; increment lands later, THIS flips to a value pin (25/20) — track that as a follow-up.
;; ON LAND (5cf911aeb): rebuild cdz; gate x3 → all (declines); pin into 14-effects; baseline x3; MR.

(case "two do-def-bound performs whose sum mixes handler-state width with a narrow param declines cleanly (not-yet-reducible, not an invalid module)"
  (input  (do
        (effect Src (op next (-> Unit Int64)))
        (def (main (: x UInt8))
          (handle Src 10
            ((next (u) s (resume s (+ s x))))
            (do
              (def a (Src.next))
              (def b (Src.next))
              (+ a b))))
        (export main)))
  (declines))
