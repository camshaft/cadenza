(case "na2 TWO sibling inner handlers both observe through ONE outer counter (shared under-frame state)"
  (input  (do
            (effect A (op a (-> Unit Int64)))
            (effect B (op b (-> Unit Int64)))
            (effect Count (op tick (-> Unit Int64)))
            (def (main (: k Int64))
              (handle Count 0 ((tick (u) c (resume c (+ c 1))))
                (+ (handle A 0 ((a (u) s (do (Count.tick) (resume 7 s)))) (A.a))
                   (+ (* 10 (handle B 0 ((b (u) s (do (Count.tick) (resume 3 s)))) (B.b)))
                      (* 100 (Count.tick))))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 237 Int64)))
