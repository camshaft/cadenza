(case "abmin3 same but NO effect-E draw — let+if on the plain parameter"
  (input  (do
            (effect A (op out (-> Int64 Int64)))
            (effect B (op bout (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle A 0
                ((out (v) t (+ 9000 v)))
                (+ (* 100 (handle B 0
                            ((bout (v) t (+ 500 v)))
                            (let ((d n))
                              (if (= (% d 3) 0) (A.out d) d))))
                   7)))
            (export main)))
  (call   main (: 3 Int64)) (output (: 9003 Int64)))
