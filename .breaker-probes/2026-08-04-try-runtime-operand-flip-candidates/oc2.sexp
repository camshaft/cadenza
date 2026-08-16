(case "oc2 the same chain via try-propagation in a helper (config-path with ?)"
  (input  (do
            (def (path (: cfg (Map Int64 (Map Int64 (List Int64)))) (: k Int64))
              (: (let ((inner (try (Map.lookup cfg 1))))
                   (let ((xs (try (Map.lookup inner 2))))
                     (let ((v (try (List.at xs 1))))
                       (Some (+ v k)))))
                 (Option Int64)))
            (def (main (: k Int64))
              (do
                (def cfg (Map.insert Map.empty 1 (Map.insert Map.empty 2 (list 10 20 30))))
                (match (path cfg k) ((Some v) v) ((None _u) -1))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 25 Int64)))
