; FINDING (breaker, 2026-07-28): TWO new faces of do-def-bound performs under a handle:
;
; F1 (wasm INVALID MODULE, rust computes 25): two performing do-defs then their sum —
;    (do (def a (Src.next)) (def b (Src.next)) (+ a b)) under (handle Src 10 ((next (u) s
;    (resume s (+ s x)))) ...) -> wasm writes an invalid module (func[0] fails validation);
;    rust computes 25 (10 + 15 with x=5). BACKEND DIVERGENCE on the wasm emit.
;    Contrast: (def f <closure>) then (+ (f 1) (f 2)) — performs INSIDE the closure — is the
;    landed 674 pin and works; the difference is the perform AS the def's INIT, twice.
;
; F2 (both backends, false unbound): ONE performing def whose binding is used inside a LATER
;    def's BIN-construction operand — (def a (Src.next)) (def frame (bin (u8 (UInt8.wrap a))))
;    -> CDZ0101 "unbound name a". Same shape with const defs works (1015). Possibly ANOTHER
;    #37-family scope hole (the handler fold lifts the performing def into a param-like slot,
;    then the bin operand's resolve misses it) — but distinct observable: F2 is unbound-at-
;    later-def-OPERAND, #37-param is unbound-after-shadow.
;
; GRADED REPRO (F1; = wasm fix pin; rust passes 25 today):
(case "two do-def-bound performs sum under their handler"
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
  (call   main (: 5 UInt8)) (output (: 25 Int64))
  (call   main (: 0 UInt8)) (output (: 20 Int64)))
