(case "s1 abort through a NON-recursive callee, pending add"
  (input  (do
            (effect Mx (op bail (-> Int64 Int64)))
            (def (f (: x Int64)) (Mx.bail x))
            (def (main)
              (+ (handle Mx 0 ((bail (v) s (* v 100))) (+ (f 5) 999999)) 7))
            (export main)))
  (call   main) (output (: 507 Int64)))
