(case "nk3 newtype Set dedupe across construction paths (direct vs computed payloads)"
  (input  (do
            (type Id (Id Int64))
            (def (main (: n Int64))
              (Set.len (Set.of (list (Id 5) (Id (+ n 4)) (Id (* n 5))))))
            (export main)))
  (call   main (: 1 Int64)) (output (: 1 Int64))
  (call   main (: 2 Int64)) (output (: 3 Int64)))
