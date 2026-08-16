(case "ql3 bisect: eval over an IF-selected quote"
  (input  (do
            (def (main (: n Int64))
              (eval (if (> n 0) (quote (+ 100 1)) (quote (- 100 3)))))
            (export main)))
  (call   main (: 1 Int64)) (output (: 101 Int64)))
