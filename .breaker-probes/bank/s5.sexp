(case "s5 abort from recursive callee with ACCUMULATOR + pending add, no ticks"
  (input  (do
            (effect Mx (op bail (-> Int64 Int64)))
            (def (go (: n Int64) (: acc Int64))
              (if (= n 0) (Mx.bail acc) (go (- n 1) (+ acc 11))))
            (def (main)
              (+ (handle Mx 0 ((bail (v) s (* v 100))) (+ (go 2 0) 999999)) 7))
            (export main)))
  (call   main) (output (: 2207 Int64)))
