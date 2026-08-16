(case "ed1 two SEPARATE effects with the SAME op name: routing by effect not op-name"
  (input  (do
            (effect A (op get (-> Unit Int64)))
            (effect B (op get (-> Unit Int64)))
            (def (main (: n Int64))
              (handle A n
                ((get (u) s (resume 100 s)))
                (handle B 0
                  ((get (u) s (resume 200 s)))
                  (+ (* 10 (A.get)) (B.get)))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 1200 Int64)))
