(case "sk3min2 Set-of-tuples state, TWO arms only (add + has)"
  (input  (do
            (effect E (op add (-> Int64)) (op has (-> Int64 Int64 Int64)))
            (def (main (: n Int64))
              (handle E (tuple n (Set.of (list)))
                ((add () st (match st
                              ((tuple s ss)
                               (resume s (tuple (+ s 2)
                                                (Set.insert ss (tuple s (+ s 1))))))))
                 (has (a b) st (match st
                                 ((tuple s ss)
                                  (resume (if (Set.contains ss (tuple a b)) 1 0) st)))))
                (do (E.add) (E.has n (+ n 1)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 1 Int64)))
