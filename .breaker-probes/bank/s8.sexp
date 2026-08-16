(case "s8 pending continuation INSIDE the recursive callee, abort deeper"
  (input  (do
            (effect Mx (op bail (-> Int64 Int64)))
            (def (go (: n Int64)) (if (= n 0) (Mx.bail 5) (+ (go (- n 1)) 999999)))
            (def (main)
              (+ (handle Mx 0 ((bail (v) s (* v 100))) (go 2)) 7))
            (export main)))
  (call   main) (output (: 507 Int64)))
