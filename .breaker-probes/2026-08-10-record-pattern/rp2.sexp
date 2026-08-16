(case "rp2 a record pattern over a Map-STORED record reads live heap fields"
  (input  (do
            (def (main (: n Int64))
              (do
                (def m (Map.insert Map.empty 1 (record (xs (list n 2)) (tag "t"))))
                (match (Map.lookup m 1)
                  ((Some r) (match r ((record (xs ys)) (List.len ys))))
                  ((None _u) -1))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 2 Int64)))
