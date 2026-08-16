(case "gp1 a perform in a GUARD-like if-condition inside recursion (condition-position perform per iteration)"
  (input  (do
            (effect Cnt (op check (-> Unit Int64)))
            (def (loop (: n Int64) (: acc Int64))
              (if (= n 0) acc
                (if (> (Cnt.check) 2)
                  (loop (- n 1) (+ acc 10))
                  (loop (- n 1) (+ acc 1)))))
            (def (main (: k Int64))
              (handle Cnt 0
                ((check (u) s (resume s (+ s 1))))
                (loop k 0)))
            (export main)))
  (call   main (: 5 Int64)) (output (: 23 Int64)))
