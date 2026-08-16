(case "abmin10 TWO nested foreign abort-only handles under the conditional abort"
  (input  (do
            (effect A (op out (-> Int64 Int64)))
            (effect B (op bout (-> Int64 Int64)))
            (effect C (op cout (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle A 0
                ((out (v) t (+ 9000 v)))
                (+ (* 100 (handle B 0
                            ((bout (v) t (+ 500 v)))
                            (+ (* 10 (handle C 0
                                       ((cout (v) t (+ 70 v)))
                                       (if (> n 0) (A.out n) n)))
                               3)))
                   7)))
            (export main)))
  (call   main (: 3 Int64)) (output (: 9003 Int64)))
