(case "la3 List.at ON a looked-up LIST with a perform-threaded index"
  (input  (do
            (effect St (op next (-> Unit Int64)))
            (def (main (: n Int64))
              (do
                (def table (Map.insert Map.empty 1 (list 10 20 30 40)))
                (handle St n
                  ((next (u) s (resume s (+ s 1))))
                  (match (Map.lookup table 1)
                    ((Some xs)
                      (match (List.at xs (St.next))
                        ((Some v) (+ v (St.next)))
                        ((None _u) -100)))
                    ((None _u) -200)))))
            (export main)))
  (call   main (: 2 Int64)) (output (: 33 Int64)))
