(case "at7 RECURSIVE performer whose resume value READS inner state (the #2102 decline floor)"
  (input  (do
            (effect A (op tick (-> Int64 Int64)))
            (effect B (op step (-> Unit Int64)))
            (def (loop (: n Int64))
              (if (= n 0) 0 (+ (B.step) (loop (- n 1)))))
            (def (main (: k Int64))
              (handle A 10
                ((tick (v) s (resume (+ s v) (+ s v))))
                (handle B 0
                  ((step (u) t (resume (A.tick (+ t 1)) (+ t 1))))
                  (+ (loop k) 100))))
            (export main)))
  (call   main (: 1 Int64)) (output (: 111 Int64)))
