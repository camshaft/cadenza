(case "xh2 CONTROL: the let-bound spelling of the cross-handler op-arg folds"
  (input  (do
            (effect A (op get (-> Unit Int64)))
            (effect B (op put (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle A n
                ((get (u) s (resume s s)))
                (handle B 0
                  ((put (v) s (resume (+ s v) (+ s v))))
                  (let ((x (A.get)))
                    (B.put x)))))
            (export main)))
  (call   main (: 7 Int64)) (output (: 7 Int64)))
