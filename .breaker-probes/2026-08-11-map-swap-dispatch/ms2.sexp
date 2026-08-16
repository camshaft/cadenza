(case "ms2 the swap's PRIOR map stays readable in the arm after the swap — persistence under the dispatch's own transition"
  (input  (do
            (effect KV (op put (-> Int64 Int64 Int64)))
            (def (main (: n Int64))
              (handle KV (Map.insert Map.empty 1 n)
                ((put (k v) s
                  (match (Map.swap s k v)
                    ((tuple _prior next)
                      (resume (+ (* 10 (match (Map.lookup s k) ((Some x) x) ((None _u) -1)))
                                 (match (Map.lookup next k) ((Some y) y) ((None _u) -2)))
                              next)))))
                (+ (KV.put 1 5) (* 1000 (KV.put 1 8)))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 58035 Int64))
  (call   main (: 0 Int64)) (output (: 58005 Int64)))
