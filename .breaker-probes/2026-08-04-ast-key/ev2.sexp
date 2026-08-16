(case "ev2 a RUNTIME-branch-selected quote evals to the selected tree's value"
  (input  (do
            (def (main (: k Int64))
              (eval (if (> k 5) (quote (* 3 4)) (quote (+ 3 4)))))
            (export main)))
  (call   main (: 9 Int64)) (error CDZ0101))
