(case "ek9 the ek8 conjunction ONE LEVEL DEEPER: middle handler arm has s-around-k AND performs outermost"
  (input  (do
            (effect A (op get (-> Unit Int64)))
            (effect B (op mid (-> Int64 Int64)))
            (effect C (op inn (-> Unit Int64)))
            (def (main (: n Int64))
              (handle A 7
                ((get (u) s (resume s s)))
                (handle B n
                  ((mid (x) s k (+ (+ s (A.get)) (k x))))
                  (handle C 0
                    ((inn (u) t (resume (B.mid 5) t)))
                    (C.inn)))))
            (export main)))
  (call   main (: 100 Int64)) (output (: 112 Int64)))
