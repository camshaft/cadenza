(case "oamin5 TWO unwraps but PURE arguments (no E) — does the sequence alone trigger it?"
  (input  (do
            (effect Bail (op out (-> Int64 Int64)))
            (def (unwrap (: o (Option Int64)) (: tag Int64))
              (match o
                ((Some v) v)
                ((None) (Bail.out tag))))
            (def (main (: n Int64))
              (handle Bail 0
                ((out (v) t (+ 500 v)))
                (let ((a (unwrap (if (> n 0) (Some n) (None)) 11)))
                  (let ((b (unwrap (if (> n -10) (Some (+ n 1)) (None)) 22)))
                    (+ (* 10 (* a b)) 3)))))
            (export main)))
  (call   main (: 4 Int64)) (output (: 203 Int64))
  (call   main (: -1 Int64)) (output (: 511 Int64))
  (call   main (: -20 Int64)) (output (: 511 Int64)))
