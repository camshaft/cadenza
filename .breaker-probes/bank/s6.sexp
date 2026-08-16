(case "s6 abort from MUTUAL recursion + pending continuation"
  (input  (do
            (effect Mx (op bail (-> Int64 Int64)))
            (def (ev (: n Int64)) (if (= n 0) (Mx.bail 5) (od (- n 1))))
            (def (od (: n Int64)) (if (= n 0) (Mx.bail 6) (ev (- n 1))))
            (def (main)
              (+ (handle Mx 0 ((bail (v) s (* v 100))) (+ (ev 2) 999999)) 7))
            (export main)))
  (call   main) (output (: 507 Int64)))
