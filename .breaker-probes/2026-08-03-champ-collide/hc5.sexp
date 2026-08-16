(case "hc5 set algebra over collision nodes — difference splits and union rebuilds the colliding pair"
  (input  (do
            (def (main (: z Int64))
              (+ (* 10 (if (= (Set.difference (Set.of (list (+ z 0) (+ z 162287980))) (Set.of (list (+ z 162287980))))
                           (Set.of (list z))) 1 0))
                 (if (= (Set.union (Set.of (list (+ z 0))) (Set.of (list (+ z 162287980))))
                        (Set.of (list (+ z 0) (+ z 162287980)))) 1 0)))
            (export main)))
  (call   main (: 1 Int64)) (output (: 11 Int64)))
