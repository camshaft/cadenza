(case "nm2 updating an INNER map re-inserts it: outer overwrite with the grown inner, old snapshot intact"
  (input  (do
            (def (main (: n Int64))
              (do
                (def inner (Map.insert Map.empty 1 n))
                (def outer1 (Map.insert Map.empty 10 inner))
                (def inner2 (Map.insert inner 2 (+ n 1)))
                (def outer2 (Map.insert outer1 10 inner2))
                (+ (* 100 (match (Map.lookup outer2 10) ((Some m) (Map.len m)) ((None _u) -1)))
                   (match (Map.lookup outer1 10) ((Some m) (Map.len m)) ((None _u) -1)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 201 Int64)))
