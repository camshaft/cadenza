(case "a branch-SELECTED Ast value splices into a quasiquote and evals per-branch"
  (input  (do
            (def (main (: b Bool))
              (let ((node (if b (quote (+ 1 2)) (quote (* 3 4)))))
                (eval (quasiquote (+ 100 (unquote node))))))
            (export main)))
  (call   main (: true Bool)) (output (: 103 Int64))
  (call   main (: false Bool)) (output (: 112 Int64)))
