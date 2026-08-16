(case "ll1 ADJACENCY-PROBE: phi-merged LIST growth in a tuple state, 2 puts + 2 reads (the slmin11 shape on a list slot)"
  (input  (do
            (effect E (op put (-> Int64)) (op size (-> Int64)))
            (def (main (: n Int64))
              (handle E (tuple n (list 7))
                ((put () st (match st
                              ((tuple s xs)
                               (resume s (tuple (+ s 1)
                                                (List.concat xs (if (= (% s 3) 0) (list 1) (list 2 3))))))))
                 (size () st (match st ((tuple s xs) (resume (List.len xs) st)))))
                (do (E.put) (E.put) (+ (E.size) (E.size)))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 8 Int64))
  (call   main (: 1 Int64)) (output (: 10 Int64))
  (call   main (: -2 Int64)) (output (: 10 Int64)))
