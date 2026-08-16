(case "lv1 SCOPE-PROBE: List of TUPLES as handler state with THREE arms (E0308-family: does the empty-list literal share the gap)"
  (input  (do
            (effect E (op push (-> Int64))
                      (op rd (-> Int64))
                      (op cnt (-> Int64)))
            (def (main (: n Int64))
              (handle E (tuple n (list))
                ((push () st (match st
                               ((tuple s xs)
                                (resume s (tuple (+ s 2)
                                                 (List.push xs (tuple s (* 2 s))))))))
                 (rd () st (match st
                             ((tuple s xs)
                              (resume (match (List.at xs 0)
                                        ((Some p) (match p ((tuple a b) (+ a b))))
                                        ((None) -1))
                                      st))))
                 (cnt () st (match st ((tuple s xs) (resume s st)))))
                (do (E.push) (E.push) (+ (E.rd) (E.cnt)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 24 Int64))
  (call   main (: 0 Int64)) (output (: 4 Int64))
  (call   main (: -3 Int64)) (output (: -8 Int64)))
