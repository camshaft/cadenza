(case "ql2 bisect: eval over a LET-BOUND match-selected quote"
  (input  (do
            (def (main (: n Int64))
              (do
                (def ast (match n
                           (0 (quote (+ 100 1)))
                           (1 (quote (* 100 2)))
                           (_ (quote (- 100 3)))))
                (eval ast)))
            (export main)))
  (call   main (: 2 Int64)) (output (: 97 Int64)))
