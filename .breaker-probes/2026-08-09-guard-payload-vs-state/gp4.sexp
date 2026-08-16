(case "gp4 a guard LADDER in the arm — two guarded arms with different state-derived thresholds classify each payload three ways"
  (input  (do
            (effect E (op classify (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle E n
                ((classify (v) s
                  (match v
                    ((guard x (> x (* 2 s))) (resume 2 (+ s 1)))
                    ((guard x (> x s)) (resume 1 (+ s 1)))
                    (_x (resume 0 s)))))
                (+ (* 10 (E.classify 8)) (E.classify 3))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 22 Int64))
  (call   main (: 4 Int64)) (output (: 10 Int64))
  (call   main (: 9 Int64)) (output (: 0 Int64)))
