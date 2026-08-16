(case "wq1 a WORKLIST algorithm: a list-driven loop draining into a visited trie (BFS discipline)"
  (input  (do
            (def (nbrs (: k Int64)) (list (* k 2) (+ (* k 2) 1)))
            (def (walk (: work (List Int64)) (: seen (Map Int64 Int64)) (: fuel Int64))
              (if (= fuel 0) seen
                (match work
                  ((list) seen)
                  ((list h .. t)
                    (match (Map.lookup seen h)
                      ((Some _v) (walk t seen (- fuel 1)))
                      ((None _u)
                        (if (> h 30) (walk t (Map.insert seen h 1) (- fuel 1))
                            (walk (List.concat t (nbrs h)) (Map.insert seen h 1) (- fuel 1)))))))))
            (def (main (: n Int64))
              (Map.len (walk (list n) Map.empty 100)))
            (export main)))
  (call   main (: 1 Int64)) (output (: 61 Int64)))
