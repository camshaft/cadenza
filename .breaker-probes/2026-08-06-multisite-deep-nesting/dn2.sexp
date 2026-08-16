(case "dn2 multi-site INNER handler, single-site outer (the inverse nesting)"
  (input  (do
            (effect Out (op bump (-> Unit Int64)))
            (effect In (op sift (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle Out 100
                ((bump (u) t (resume t (+ t 10))))
                (handle In 0
                  ((sift (v) s (if (> v 10) (resume v (+ s 1)) (resume 0 s))))
                  (+ (In.sift 20) (+ (Out.bump) (In.sift n))))))
            (export main)))
  (call   main (: 30 Int64)) (output (: 150 Int64)))
