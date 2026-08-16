(case "mv11v multi-shot + let + n as handle seed"
  (input  (do
            (effect Amb (op pick (-> Unit Int64)))
            (def (main (: n Int64))
              (handle Amb n
                ((pick (u) s (+ (resume (+ s 1) s) (resume 2 s))))
                (let ((x (Amb.pick)))
                  (+ (* 10 x) x))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 88 Int64)))
