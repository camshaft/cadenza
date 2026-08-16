(case "na1 an arm performs the outer effect TWICE and its RESULTS feed the resume value"
  (input  (do
            (effect A (op a (-> Unit Int64)))
            (effect Count (op tick (-> Unit Int64)))
            (def (main (: k Int64))
              (handle Count 10 ((tick (u) c (resume c (+ c 1))))
                (+ (handle A 0 ((a (u) s (resume (+ (Count.tick) (Count.tick)) s))) (A.a))
                   (* 100 (Count.tick)))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 1221 Int64)))
