(case "sq2 a Map keyed by a record whose field is a SET (triple-nested CHAMP: map<record<set>>)"
  (input  (do
            (def (main (: n Int64))
              (match (Map.lookup (Map.insert Map.empty (record (s (Set.of (list 1 2))) (id 7)) 42)
                                 (record (s (Set.of (list n 1))) (id 7)))
                ((Some v) v) ((None _u) -1)))
            (export main)))
  (call   main (: 2 Int64)) (output (: 42 Int64))
  (call   main (: 3 Int64)) (output (: -1 Int64)))
