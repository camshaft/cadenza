(case "uw1 List.update WRITES a compound value into an RRB slot and reads it back at depth"
  (input  (do
            (def (build (: i Int64) (: acc (List (Tuple Int64 Int64))))
              (if (= i 0) acc (build (- i 1) (List.push acc (tuple i (* i 2))))))
            (def (main (: n Int64))
              (do
                (def xs (build n (list)))
                (def ys (List.update xs 20 (tuple 999 888)))
                (+ (* 10 (match (List.at ys 20) ((Some p) (match p ((tuple a b) (+ a b)))) ((None _u) -1)))
                   (match (List.at xs 20) ((Some p) (match p ((tuple a _b) (if (= a 999) 0 1)))) ((None _u) -1)))))
            (export main)))
  (call   main (: 60 Int64)) (output (: 18871 Int64)))
