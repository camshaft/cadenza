(case "oc1 an Option chain through nested Map lookups (the config-path idiom via match nesting)"
  (input  (do
            (def (main (: k Int64))
              (do
                (def cfg (Map.insert Map.empty 1 (Map.insert Map.empty 2 (list 10 20 30))))
                (match (Map.lookup cfg 1)
                  ((Some inner)
                    (match (Map.lookup inner 2)
                      ((Some xs) (match (List.at xs k) ((Some v) v) ((None _u) -3)))
                      ((None _u) -2)))
                  ((None _u) -1))))
            (export main)))
  (call   main (: 1 Int64)) (output (: 20 Int64))
  (call   main (: 9 Int64)) (output (: -3 Int64)))
