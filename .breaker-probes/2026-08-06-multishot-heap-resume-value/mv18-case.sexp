(case "mv18c multi-shot + n + match-binder consumer (no let)"
  (input  (do
            (effect Amb (op pick (-> Unit Int64)))
            (def (main (: n Int64))
              (handle Amb 0
                ((pick (u) s (+ (resume (+ n 1) s) (resume 2 s))))
                (match (Amb.pick) (v (+ (* 10 v) v)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 88 Int64)))
