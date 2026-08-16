(case "ag1 an abortive arm reads a MAP state GROWN by three earlier dispatches — abort sees the accumulated heap, not the seed"
  (input  (do
            (effect St
              (op put (-> Int64 Int64))
              (op halt (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St Map.empty
                ((put (v) s (resume (Map.len s) (Map.insert s v (* v 10))))
                 (halt (u) s
                  (+ (* 100 (Map.len s))
                     (match (Map.lookup s n) ((Some v) v) ((None u2) -5)))))
                (match (St.put 1)
                  (_ (match (St.put 2)
                       (_ (match (St.put n)
                            (_ (+ 7777 (St.halt))))))))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 330 Int64))
  (call   main (: 2 Int64)) (output (: 220 Int64)))
