(case "gd4 pure guards INSIDE the handler arm grade the live state — three dispatches cross the tier boundaries as the state climbs"
  (input  (do
            (effect E (op grade (-> Int64)))
            (def (main (: n Int64))
              (handle E n
                ((grade () s (resume (match s
                                       ((guard v (> v 10)) 3)
                                       ((guard v (> v 0)) 2)
                                       (_v 1))
                                     (+ s 4))))
                (+ (* 100 (E.grade)) (+ (* 10 (E.grade)) (E.grade)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 223 Int64))
  (call   main (: -2 Int64)) (output (: 122 Int64))
  (call   main (: 11 Int64)) (output (: 333 Int64)))
