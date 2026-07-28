; FINDING (breaker, 2026-07-21): the List.len-of-a-literal fold DROPS a TRAPPING element
; construction — numeric-model.md #A Rational With A Zero Denominator Is Not A Value says the
; construction "MUST fail at a defined point rather than produce a value", but:
;
;   (List.len (list (Rational.of 1 2) (Rational.of 3 d)))  at runtime d = 0
;     → RUNS to 2 on wasm AND rust AND rust-async, O0..O3   [expected: trap "unreachable"]
;
; Control faces (all behave correctly):
;   (Rational.truncate (Rational.of 3 d)) at d=0                    → traps (39f2e6edb's guard)
;   observing the element via List.at + expect + truncate at d=0    → traps
;   (Rational.of 5 (- a b)) at a=b, result CONSUMED                 → traps
;
; Root shape: List.len over a 2-element list LITERAL is const-foldable to 2 without reading
; element values — but the second element's construction can TRAP at runtime (zero denominator
; guard), so folding the whole expression away VIOLATES trap preservation (the is_trap_free
; discipline that LICM/fold already applies to checked arithmetic; a Rational.of with a RUNTIME
; denominator is not trap-free). Same outcome at every opt level and on all three backends, so
; the drop is in the shared Core-level fold, not a backend.
;
; Same class as the landed reassociation trap-preservation pins (a cancelling chain keeps its
; intermediate overflow): an elision that changes a trapping program into a non-trapping one is
; not meaning-preserving. Expected: trap at d=0, 2 at d=4.

(case "REPRO List.len over a list literal keeps a trapping element construction"
  (input  (do
            (def (main (: d Int64))
              (List.len (list (Rational.of 1 2) (Rational.of 3 d))))
            (export main)))
  (call   main (: 4 Int64)) (output (: 2 Int64))
  (call   main (: 0 Int64)) (trap "unreachable"))

(case "CONTROL the consumed element traps today"
  (input  (do
            (def (main (: d Int64))
              (Rational.truncate (Option.expect (List.at (list (Rational.of 1 2) (Rational.of 3 d)) 1) "x")))
            (export main)))
  (call   main (: 0 Int64)) (trap "unreachable"))
