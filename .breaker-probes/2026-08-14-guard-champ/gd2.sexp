(case "gd2 a match GUARD performing a Map lookup on the pattern binder (guard x CHAMP)"
  (input  (do
            (def (main (: k Int64))
              (do
                (def prices (Map.insert (Map.insert Map.empty 1 100) 2 50))
                (match (Some k)
                  ((guard (Some id) (match (Map.lookup prices id) ((Some p) (> p 60)) ((None _u) false))) 1)
                  ((Some _id) 2)
                  ((None _u) -1))))
            (export main)))
  (call   main (: 1 Int64)) (output (: 1 Int64))
  (call   main (: 2 Int64)) (output (: 2 Int64))
  (call   main (: 9 Int64)) (output (: 2 Int64)))
