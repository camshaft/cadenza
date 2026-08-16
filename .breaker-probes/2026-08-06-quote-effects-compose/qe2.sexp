(case "qe2 a handler arm CONSTRUCTS an Ast from the perform-time state and the body evals it"
  (input  (do
            (effect St (op mk (-> Unit Ast)))
            (def (main (: n Int64))
              (handle St n
                ((mk (u) s (resume (Ast.Int (BigInt.of s)) (+ s 1))))
                (+ (eval (St.mk)) (* 100 (eval (St.mk))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 605 Int64)))
