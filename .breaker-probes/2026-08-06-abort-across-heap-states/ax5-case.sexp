(case "ax5 the NO-LET face: inner handle result consumed inline by a strict operand"
  (input  (do
            (effect A (op tick (-> Unit Int64)) (op get (-> Unit Int64)))
            (effect B (op bail (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle A 10
                ((tick (u) s (resume s (+ s 1))) (get (u) s (resume s s)))
                (+ (* 10 (handle B 0 ((bail (v) s v)) (+ (A.tick) (B.bail 99))))
                   (A.get))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 1001 Int64)))
