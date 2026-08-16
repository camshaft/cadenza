(case "s7 abort at recursion DEPTH ZERO (base case immediate) + pending continuation"
  (input  (do
            (effect Mx (op bail (-> Int64 Int64)))
            (def (go (: n Int64)) (if (= n 0) (Mx.bail 5) (go (- n 1))))
            (def (main)
              (+ (handle Mx 0 ((bail (v) s (* v 100))) (+ (go 0) 999999)) 7))
            (export main)))
  (call   main) (output (: 507 Int64)))
