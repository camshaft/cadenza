(case "cv4 the closure STATE's body performs a SECOND effect when the arm applies it"
  (input  (do
            (effect Aux (op base (-> Unit Int64)))
            (effect St (op eval (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle Aux 50
                ((base (u) b (resume b (+ b 1))))
                (handle St (fn ((: x Int64)) (+ x (Aux.base)))
                  ((eval (v) f (resume (f v) f)))
                  (+ (* 100 (St.eval n)) (St.eval 3)))))
            (export main)))
  (call   main (: 4 Int64)) (output (: 5454 Int64)))
