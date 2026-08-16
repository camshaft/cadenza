(case "ms12 helper-built map: Map.empty flows through a pure helper that inserts — evidence at a distance"
  (input  (do
            (def (stash (: m (Map String Int64)) (: v Int64)) (Map.insert m "k" v))
            (def (main (: n Int64))
              (let ((m (stash Map.empty n)))
                (match (Map.lookup m "k") ((Some x) x) ((None _u) -1))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 5 Int64)))
