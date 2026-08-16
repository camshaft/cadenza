(case "st2 Set state where the ARM branches on membership (visited-set gate inside the arm)"
  (input  (do
            (effect St (op visit (-> Int64 Int64)))
            (def (main (: a Int64))
              (handle St (Set.of (list))
                ((visit (v) s
                  (if (Set.contains s v)
                    (resume 0 s)
                    (resume 1 (Set.insert s v)))))
                (+ (* 100 (St.visit a))
                   (+ (* 10 (St.visit a))
                      (St.visit (+ a 1))))))
            (export main)))
  (call   main (: 7 Int64)) (output (: 101 Int64)))
