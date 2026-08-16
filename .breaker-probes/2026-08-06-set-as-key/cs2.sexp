(case "cs2 direct < over runtime Sets"
  (input  (do
            (def (main (: k Int64))
              (if (< (Set.of (list 1 k)) (Set.of (list 1 3))) 1 0))
            (export main)))
  (call   main (: 2 Int64)) (output (: 1 Int64)))
