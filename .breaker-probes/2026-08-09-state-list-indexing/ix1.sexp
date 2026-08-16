(case "ix1 the op argument INDEXES a list held in a two-slot state — in-range reads project, out-of-range yields the arm's fallback"
  (input  (do
            (effect E (op at (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle E (tuple (list 10 20 30 40) n)
                ((at (u) s (match s
                             ((tuple xs k)
                               (resume (match (List.at xs (% k 6))
                                         ((Some v) v)
                                         ((None) -1))
                                       (tuple xs (+ k 2)))))))
                (+ (* 100 (E.at 0)) (E.at 0))))
            (export main)))
  (call   main (: 1 Int64)) (output (: 2040 Int64))
  (call   main (: 4 Int64)) (output (: -90 Int64))
  (call   main (: 3 Int64)) (output (: 3999 Int64)))
