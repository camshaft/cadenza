(case "lv2 CONTROL: NON-EMPTY seeded list of tuples (element type solved at the seed) with the same three arms"
  (input  (do
            (effect E (op push (-> Int64))
                      (op rd (-> Int64))
                      (op cnt (-> Int64)))
            (def (main (: n Int64))
              (handle E (tuple n (list (tuple 0 0)))
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
                (do (E.push) (+ (E.rd) (E.cnt)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 7 Int64))
  (call   main (: 0 Int64)) (output (: 2 Int64)))
