(case "na3 a DEPTH-3 arm-perform chain: C's arm performs B, whose arm performs A (transitive under-frame)"
  (input  (do
            (effect A (op a (-> Unit Int64)))
            (effect B (op b (-> Unit Int64)))
            (effect C (op c (-> Unit Int64)))
            (def (main (: k Int64))
              (handle A 100 ((a (u) s (resume s (+ s 1))))
                (handle B 0 ((b (u) s (resume (A.a) s)))
                  (handle C 0 ((c (u) s (resume (* 10 (B.b)) s)))
                    (+ (C.c) (+ (C.c) (A.a)))))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 2112 Int64)))
