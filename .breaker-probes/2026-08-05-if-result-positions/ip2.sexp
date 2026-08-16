(case "ip2 a handle as a FUNCTION ARGUMENT ((helper (handle ...)) — handle in arg position)"
  (input  (do
            (effect A (op a (-> Unit Int64)))
            (def (triple (: x Int64)) (* 3 x))
            (def (main (: n Int64))
              (triple (handle A n ((a (u) s (resume (+ s 1) s))) (A.a))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 18 Int64)))
