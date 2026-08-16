(case "cs3 a SET as a Map key (needs the internal total order) — hits by content"
  (input  (do
            (def (main (: k Int64))
              (match (Map.lookup (Map.insert Map.empty (Set.of (list 1 2)) 42) (Set.of (list k 1)))
                ((Some v) v) ((None _u) -1)))
            (export main)))
  (call   main (: 2 Int64)) (output (: 42 Int64))
  (call   main (: 3 Int64)) (output (: -1 Int64)))
