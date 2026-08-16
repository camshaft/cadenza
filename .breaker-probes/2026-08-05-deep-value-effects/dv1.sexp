(case "dv1 a 3-level nested compound (Map of tuples of Lists) built via performs, probed at full depth"
  (input  (do
            (effect St (op a (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St n
                ((a (u) s (resume s (+ s 1))))
                (do
                  (def m (Map.insert Map.empty 1 (tuple (St.a) (list (St.a) (St.a)))))
                  (match (Map.lookup m 1)
                    ((Some t) (+ (* 100 (. t 0))
                                 (match (List.at (. t 1) 1) ((Option.Some v) v) ((Option.None) -1))))
                    ((None _u) -2)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 507 Int64)))
