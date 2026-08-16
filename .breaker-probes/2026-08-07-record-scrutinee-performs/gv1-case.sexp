(case "gv1 the LIST-literal scrutinee sibling — draws fire once and bind by position"
  (input  (do
            (effect St (op next (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St n
                ((next (u) s (resume s (+ s 1))))
                (match (list (St.next) (St.next))
                  ((list x y) (+ (* 10 x) y))
                  (_other -1))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 56 Int64)))
