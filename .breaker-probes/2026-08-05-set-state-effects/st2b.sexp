(case "st2b control: same membership-branching arm, ONE perform"
  (input  (do
            (effect St (op visit (-> Int64 Int64)))
            (def (main (: a Int64))
              (handle St (Set.of (list))
                ((visit (v) s
                  (if (Set.contains s v)
                    (resume 0 s)
                    (resume 1 (Set.insert s v)))))
                (St.visit a)))
            (export main)))
  (call   main (: 7 Int64)) (output (: 1 Int64)))
