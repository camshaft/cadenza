(case "he2 a LIST OF BIGINTS as op ARGUMENT — the arm folds heap-numeric elements it is handed"
  (input  (do
            (effect St (op total (-> (List BigInt) Int64)))
            (def (sum-b (: xs (List BigInt)) (: i Int64) (: acc BigInt))
              (match (List.at xs i)
                ((Some v) (sum-b xs (+ i 1) (+ acc v)))
                ((None _u) acc)))
            (def (main (: n Int64))
              (handle St 0
                ((total (xs) s (resume (Int64.of (sum-b xs 0 (BigInt.of 0))) s)))
                (St.total (list (BigInt.of n) (BigInt.of 100) (BigInt.of 3000)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 3105 Int64)))
