(case "abmin13 the foreign conditional abort lives in a DEF called from A's body — cross-function face of the mis-homing"
  (input  (do
            (effect A (op out (-> Int64 Int64)))
            (effect B (op bout (-> Int64 Int64)))
            (def (inner (: n Int64))
              (handle B 0
                ((bout (v) t (+ 500 v)))
                (if (> n 0) (A.out n) n)))
            (def (main (: n Int64))
              (handle A 0
                ((out (v) t (+ 9000 v)))
                (+ (* 100 (inner n)) 7)))
            (export main)))
  (call   main (: 3 Int64)) (output (: 9003 Int64))
  (call   main (: -2 Int64)) (output (: -193 Int64)))
