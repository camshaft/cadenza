(case "mv7v multi-shot + let + n in resume value"
  (input  (do
            (effect Amb (op pick (-> Unit Int64)))
            (def (main (: n Int64))
              (handle Amb 0
                ((pick (u) s (+ (resume (+ n 1) s) (resume 2 s))))
                (let ((x (Amb.pick)))
                  (+ (* 10 x) x))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 88 Int64)))
