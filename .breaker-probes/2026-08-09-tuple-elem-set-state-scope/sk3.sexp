(case "sk3 SCOPE-PROBE: Set of TUPLES as handler state with THREE arms (E0308-family adjacency)"
  (input  (do
            (effect E (op add (-> Int64))
                      (op has (-> Int64 Int64 Int64))
                      (op cnt (-> Int64)))
            (def (main (: n Int64))
              (handle E (tuple n (Set.of (list)))
                ((add () st (match st
                              ((tuple s ss)
                               (resume s (tuple (+ s 2)
                                                (Set.insert ss (tuple s (+ s 1))))))))
                 (has (a b) st (match st
                                 ((tuple s ss)
                                  (resume (if (Set.contains ss (tuple a b)) 1 0) st))))
                 (cnt () st (match st ((tuple s ss) (resume s st)))))
                (do (E.add) (E.add)
                    (+ (E.has n (+ n 1))
                       (+ (* 1000 (E.has (+ n 9) (+ n 10)))
                          (E.cnt))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 10 Int64))
  (call   main (: 0 Int64)) (output (: 5 Int64))
  (call   main (: -3 Int64)) (output (: 2 Int64)))
