(case "oc3 bisect: try over ONE Map.lookup with const key"
  (input  (do
            (def (one (: cfg (Map Int64 Int64)))
              (: (let ((v (try (Map.lookup cfg 1)))) (Some (+ v 1))) (Option Int64)))
            (def (main (: k Int64))
              (match (one (Map.insert Map.empty 1 k)) ((Some v) v) ((None _u) -1)))
            (export main)))
  (call   main (: 5 Int64)) (output (: 6 Int64)))
