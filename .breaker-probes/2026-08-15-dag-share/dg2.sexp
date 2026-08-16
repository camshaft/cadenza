(case "dg2 the shared value RE-READ through each container AFTER the growth (no aliased mutation leaked)"
  (input  (do
            (def (main (: n Int64))
              (do
                (def shared (list n))
                (def m (Map.insert (Map.insert Map.empty 1 shared) 2 shared))
                (def g1 (match (Map.lookup m 1) ((Some xs) (List.push xs 7)) ((None _u) (list))))
                (def g2 (match (Map.lookup m 2) ((Some xs) (List.push xs 8)) ((None _u) (list))))
                (+ (* 1000 (match (List.at g1 1) ((Some v) v) ((None _u) -1)))
                   (+ (* 10 (match (List.at g2 1) ((Some v) v) ((None _u) -1)))
                      (match (Map.lookup m 1) ((Some xs) (List.len xs)) ((None _u) -1))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 7081 Int64)))
