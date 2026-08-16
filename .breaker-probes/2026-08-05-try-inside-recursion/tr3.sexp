(case "tr3 abort in the recursion but the RECURSIVE CALL is effect-free (halt guards entry, loop body pure)"
  (input  (do
            (effect Cnt (op halt (-> Unit Int64)))
            (def (pure-loop (: n Int64))
              (if (= n 0) 0 (+ 1 (pure-loop (- n 1)))))
            (def (main (: n Int64))
              (handle Cnt 5
                ((halt (u) s (* 1000 s)))
                (if (> n 3) (Cnt.halt) (pure-loop n))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 5000 Int64)))
