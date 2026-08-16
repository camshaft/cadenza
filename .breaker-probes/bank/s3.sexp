(case "s3 abort through a RECURSIVE callee, NO pending continuation"
  (input  (do
            (effect Mx (op bail (-> Int64 Int64)))
            (def (go (: n Int64)) (if (= n 0) (Mx.bail 5) (go (- n 1))))
            (def (main)
              (+ (handle Mx 0 ((bail (v) s (* v 100))) (go 2)) 7))
            (export main)))
  (call   main) (output (: 507 Int64)))
