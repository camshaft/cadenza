(case "su1f a fold UNIONING 20 singleton sets equals the direct 20-element build"
  (input  (do
            (def (uall (: i Int64) (: acc (Set Int64)))
              (if (= i 0) acc (uall (- i 1) (Set.union acc (Set.of (list i))))))
            (def (build (: i Int64) (: acc (Set Int64)))
              (if (= i 0) acc (build (- i 1) (Set.insert acc i))))
            (def (main (: n Int64))
              (do
                (def via-union (uall n (Set.of (list 0))))
                (def direct (build n (Set.of (list 0))))
                (+ (* 10 (if (= via-union direct) 1 0)) (Set.len via-union))))
            (export main)))
  (call   main (: 20 Int64)) (output (: 31 Int64)))
