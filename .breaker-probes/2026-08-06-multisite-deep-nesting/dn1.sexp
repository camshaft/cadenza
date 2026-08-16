(case "dn1 a multi-site arm on the OUTER of two nested handlers (inner single-site, separate effects)"
  (input  (do
            (effect Out (op sift (-> Int64 Int64)))
            (effect In (op bump (-> Unit Int64)))
            (def (main (: n Int64))
              (handle Out 0
                ((sift (v) s (if (> v 10) (resume v (+ s 1)) (resume 0 s))))
                (handle In 100
                  ((bump (u) t (resume t (+ t 10))))
                  (+ (Out.sift 20) (+ (In.bump) (Out.sift n))))))
            (export main)))
  (call   main (: 30 Int64)) (output (: 150 Int64)))
