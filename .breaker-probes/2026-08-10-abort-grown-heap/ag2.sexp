(case "ag2 a conditionally-aborting arm reads a MAP state GROWN by its own earlier dispatches — the abort leg sees the accumulated heap"
  (input  (do
            (effect St (op put (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle St Map.empty
                ((put (v) s
                  (if (= v 0)
                      (+ (* 100 (Map.len s))
                         (match (Map.lookup s n) ((Some w) w) ((None _u) -5)))
                      (resume (Map.len s) (Map.insert s v (* v 10))))))
                (match (St.put 1)
                  (_ (match (St.put 2)
                       (_ (match (St.put n)
                            (_ (+ 7777 (St.put 0))))))))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 330 Int64))
  (call   main (: 2 Int64)) (output (: 220 Int64)))
