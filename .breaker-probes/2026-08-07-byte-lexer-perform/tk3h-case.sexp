(case "tk3h pump then a SCALAR-state let-bound dispatch (Int64 state, not Bytes)"
  (input  (do
            (effect Ctr (op bump (-> Unit Unit)) (op get (-> Unit Int64)))
            (def (pump (: k Int64))
              (if (= k 0) unit
                  (do (Ctr.bump) (pump (- k 1)))))
            (def (main (: n Int64))
              (handle Ctr 0
                ((bump (u) s (resume unit (+ s 1)))
                 (get (u) s (resume s s)))
                (do
                  (pump 3)
                  (let ((v (Ctr.get)))
                    (+ v 100)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 103 Int64)))
