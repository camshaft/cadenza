(case "gp2 perform in a MATCH SCRUTINEE inside recursion (scrutinee-position per iteration)"
  (input  (do
            (effect Cnt (op check (-> Unit Int64)))
            (def (loop (: n Int64) (: acc Int64))
              (if (= n 0) acc
                (match (% (Cnt.check) 2)
                  (0 (loop (- n 1) (+ acc 1)))
                  (_ (loop (- n 1) (+ acc 10))))))
            (def (main (: k Int64))
              (handle Cnt 0
                ((check (u) s (resume s (+ s 1))))
                (loop k 0)))
            (export main)))
  (call   main (: 4 Int64)) (output (: 22 Int64)))
