(case "ax4 the NESTED-operand face: (+ 999 (+ (A.tick) (B.bail 99)))"
  (input  (do
            (effect A (op tick (-> Unit Int64)) (op get (-> Unit Int64)))
            (effect B (op bail (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle A 10
                ((tick (u) s (resume s (+ s 1))) (get (u) s (resume s s)))
                (let ((b (handle B 0 ((bail (v) s v)) (+ 999 (+ (A.tick) (B.bail 99))))))
                  (+ b (A.get)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 110 Int64)))
