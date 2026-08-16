(case "xe3 depth-3: innermost perform's arg reaches across TWO handler layers"
  (input  (do
            (effect A (op get (-> Unit Int64)))
            (effect B (op mid (-> Int64 Int64)))
            (effect C (op inn (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle A n
                ((get (u) s (resume s s)))
                (handle B 100
                  ((mid (v) s (resume (+ v s) s)))
                  (handle C 0
                    ((inn (v) s (resume (* 2 v) s)))
                    (C.inn (B.mid (A.get)))))))
            (export main)))
  (call   main (: 7 Int64)) (output (: 214 Int64)))
