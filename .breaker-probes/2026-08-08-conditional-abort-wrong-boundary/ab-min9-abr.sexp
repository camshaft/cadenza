(case "abmin9 the RESUMPTIVE flip of abmin4 — A's arm resumes, so 900307 IS the correct answer here"
  (input  (do
            (effect A (op out (-> Int64 Int64)))
            (effect B (op bout (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle A 0
                ((out (v) t (resume (+ 9000 v) t)))
                (+ (* 100 (handle B 0
                            ((bout (v) t (+ 500 v)))
                            (if (> n 0) (A.out n) n)))
                   7)))
            (export main)))
  (call   main (: 3 Int64)) (output (: 900307 Int64))
  (call   main (: -2 Int64)) (output (: -193 Int64)))
