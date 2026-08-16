(case "tb3 the WORKING side: try chains List.at reads with indices from plain matches"
  (input  (do
            (def (fill (: i Int64) (: m (Map Int64 Int64)))
              (if (= i 0) m (fill (- i 1) (Map.insert m i (+ i 1)))))
            (def (pick (: xs (List Int64)) (: idx Int64))
              (let ((v (try (List.at xs idx))))
                (Some (* v 10))))
            (def (main (: n Int64))
              (do
                (def m (fill n Map.empty))
                (def xs (list 10 20 30))
                (def idx (match (Map.lookup m 1) ((Some v) v) ((None _u) 99)))
                (+ (* 10 (match (pick xs idx) ((Some v) v) ((None _u) -1)))
                   (match (pick xs 99) ((Some _v) 0) ((None _u) 1)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 2001 Int64)))
