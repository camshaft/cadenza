(case "ah2 the ABORT arm runs the recursive pure computation on its state (heavy abort face)"
  (input  (do
            (effect St (op bump (-> Unit Int64)) (op halt (-> Unit Int64)))
            (def (fib (: n Int64))
              (if (< n 2) n (+ (fib (- n 1)) (fib (- n 2)))))
            (def (main (: n Int64))
              (handle St n
                ((bump (u) s (resume 0 (+ s 1)))
                 (halt (u) s (fib s)))
                (+ (* 0 (St.bump)) (St.halt))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 89 Int64)))
