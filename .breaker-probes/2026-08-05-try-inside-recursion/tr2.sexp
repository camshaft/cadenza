(case "tr2 the wp4 shape with the abort in TAIL position of the recursion (halt as the deepest call, no + around it)"
  (input  (do
            (effect Cnt (op bump (-> Unit Int64)) (op halt (-> Unit Int64)))
            (def (loop (: n Int64))
              (if (= n 0) (Cnt.halt) (do (def _x (Cnt.bump)) (loop (- n 1)))))
            (def (main (: n Int64))
              (handle Cnt 0
                ((bump (u) s (resume s (+ s 1)))
                 (halt (u) s (* 1000 s)))
                (loop n)))
            (export main)))
  (call   main (: 2 Int64)) (output (: 2000 Int64)))
