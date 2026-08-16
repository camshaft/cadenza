(case "abmin4 if WITHOUT let"
  (input  (do
            (effect A (op out (-> Int64 Int64)))
            (effect B (op bout (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle A 0
                ((out (v) t (+ 9000 v)))
                (+ (* 100 (handle B 0
                            ((bout (v) t (+ 500 v)))
                            (if (= (% n 3) 0) (A.out n) n)))
                   7)))
            (export main)))
  (call   main (: 3 Int64)) (output (: 9003 Int64)))
