(case "sq3 QUADRUPLE nesting: a Set of maps-of-sets dedupes by full-depth content"
  (input  (do
            (def (main (: n Int64))
              (Set.len (Set.of (list
                (Map.insert Map.empty (Set.of (list 1 n)) "v")
                (Map.insert Map.empty (Set.of (list 2 1)) "v")
                (Map.insert Map.empty (Set.of (list 1 2)) "w")))))
            (export main)))
  (call   main (: 2 Int64)) (output (: 2 Int64))
  (call   main (: 3 Int64)) (output (: 3 Int64)))
