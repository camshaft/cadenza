(case "abmin7 conditional abort under a RESUMPTIVE inner handler of a different effect"
  (input  (do
            (effect A (op out (-> Int64 Int64)))
            (effect R (op get (-> Int64)))
            (def (main (: n Int64))
              (handle A 0
                ((out (v) t (+ 9000 v)))
                (+ (* 100 (handle R 5
                            ((get () t (resume t t)))
                            (if (> n 0) (A.out n) (R.get))))
                   7)))
            (export main)))
  (call   main (: 3 Int64)) (output (: 9003 Int64))
  (call   main (: -2 Int64)) (output (: 507 Int64)))
