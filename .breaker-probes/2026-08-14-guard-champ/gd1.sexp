(case "gd1 a guard reading a MAP lookup on the scrutinee's own payload (guard × CHAMP composition)"
  (input  (do
            (def (main (: k Int64))
              (do
                (def prices (Map.insert (Map.insert Map.empty 1 100) 2 50))
                (match (Some k)
                  ((Some id) (if (match (Map.lookup prices id) ((Some p) (> p 60)) ((None _u) false))
                                 1
                                 (if (= id 2) 2 0)))
                  (None -1))))
            (export main)))
  (call   main (: 1 Int64)) (output (: 1 Int64))
  (call   main (: 2 Int64)) (output (: 2 Int64))
  (call   main (: 9 Int64)) (output (: 0 Int64)))
