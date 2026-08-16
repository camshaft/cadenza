(case "su2f a fold INTERSECTING a shrinking chain converges to the common core"
  (input  (do
            (def (isect (: i Int64) (: acc (Set Int64)))
              (if (= i 0) acc
                  (isect (- i 1) (Set.intersection acc (Set.of (list 1 2 3 i (+ i 10)))))))
            (def (main (: n Int64))
              (do
                (def core (isect n (Set.of (list 1 2 3 99 100))))
                (+ (* 10 (Set.len core)) (if (Set.contains core 2) 1 0))))
            (export main)))
  (call   main (: 8 Int64)) (output (: 31 Int64)))
