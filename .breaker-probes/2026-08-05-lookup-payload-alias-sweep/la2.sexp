(case "la2 Map.lookup ON a looked-up MAP with a perform-threaded key (nested lookup)"
  (input  (do
            (effect St (op next (-> Unit Int64)))
            (def (main (: n Int64))
              (do
                (def outer (Map.insert Map.empty 1 (Map.insert (Map.insert Map.empty 1 100) 2 250)))
                (handle St n
                  ((next (u) s (resume s (+ s 1))))
                  (match (Map.lookup outer 1)
                    ((Some inner)
                      (match (Map.lookup inner (St.next))
                        ((Some v) (+ v (St.next)))
                        ((None _u) -100)))
                    ((None _u) -200)))))
            (export main)))
  (call   main (: 2 Int64)) (output (: 253 Int64)))
