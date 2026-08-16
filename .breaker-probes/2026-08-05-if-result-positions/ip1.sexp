(case "ip1 a handle expression as an IF-BRANCH value (handle in branch position, both branches handles)"
  (input  (do
            (effect A (op a (-> Unit Int64)))
            (def (main (: n Int64))
              (+ 1 (if (> n 3)
                     (handle A n ((a (u) s (resume (* s 10) s))) (A.a))
                     (handle A n ((a (u) s (resume (* s 100) s))) (A.a)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 51 Int64)))
