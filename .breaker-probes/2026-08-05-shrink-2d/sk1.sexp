(case "sk1 a shrinker whose predicate needs BOTH a value floor AND a length floor (2D minimality)"
  (input  (do
        (def (sum-l (: xs (List Int64)) (: acc Int64))
          (match xs
            ((list) acc)
            ((list h .. t) (sum-l t (+ acc h)))))
        (def (fails (: xs (List Int64)))
          (and (>= (sum-l xs 0) 100) (>= (List.len xs) 2)))
        (def (drop-at (: xs (List Int64)) (: i Int64) (: j Int64) (: acc (List Int64)))
          (match xs
            ((list) acc)
            ((list h .. t)
              (if (= j i)
                  (drop-at t i (+ j 1) acc)
                  (drop-at t i (+ j 1) (List.push acc h))))))
        (def (try-drops (: xs (List Int64)) (: i Int64))
          (if (>= i (List.len xs))
              xs
              (do
                (def cand (drop-at xs i 0 (list)))
                (if (fails cand)
                    (try-drops cand 0)
                    (try-drops xs (+ i 1))))))
        (def (main (: mode Int64))
          (do
            (def xs (list 90 60 50 30))
            (def m (try-drops xs 0))
            (+ (* (sum-l m 0) 10) (List.len m))))
        (export main)))
  (call   main (: 1 Int64)) (output (: 1102 Int64)))
