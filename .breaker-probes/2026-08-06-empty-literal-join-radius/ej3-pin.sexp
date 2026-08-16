(case "an empty MAP fallback beside an unsolved-Var arm grounds — the Map-of-Maps face"
  (input  (do
            (def (main (: n Int64))
              (let ((m Map.empty))
                (let ((inner (match (Map.lookup m "k") ((Some ys) ys) ((None _u) Map.empty))))
                  (Map.len (Map.insert inner "x" n)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 1 Int64)))
