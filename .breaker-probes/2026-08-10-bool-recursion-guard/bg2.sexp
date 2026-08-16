(case "bg2 the recursion guard is an AND of a bool draw and a pure bound check — short-circuit must not skip the draw's state advance observation"
  (input  (do
            (effect T (op odd (-> Bool)) (op tick (-> Int64)))
            (def (walk (: k Int64) (: acc Int64))
              (if (and (< k 6) (T.odd))
                  (walk (+ k 1) (+ (* 10 acc) (T.tick)))
                  acc))
            (def (main (: n Int64))
              (handle T n
                ((odd () s (resume (= (% s 2) 1) (+ s 1)))
                 (tick () s (resume s (+ s 1))))
                (+ (* 100 (walk 0 0)) (T.tick))))
            (export main)))
  (call   main (: 1 Int64)) (output (: 24691213 Int64))
  (call   main (: 2 Int64)) (output (: 3 Int64)))
