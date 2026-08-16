(case "tb2 bisect: try over Map.lookup ALONE (no List.at chain)"
  (input  (do
            (def (fill (: i Int64) (: m (Map Int64 Int64)))
              (if (= i 0) m (fill (- i 1) (Map.insert m i (+ i 1)))))
            (def (pick (: m (Map Int64 Int64)) (: k Int64))
              (let ((v (try (Map.lookup m k))))
                (Some (* v 10))))
            (def (main (: n Int64))
              (do
                (def m (fill n Map.empty))
                (+ (* 10 (match (pick m 1) ((Some v) v) ((None _u) -1)))
                   (match (pick m 99) ((Some _v) 0) ((None _u) 1)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 201 Int64)))
