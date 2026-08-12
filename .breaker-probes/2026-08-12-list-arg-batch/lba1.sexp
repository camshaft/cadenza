(case "lba1 an op taking a LIST argument — the arm folds the batch into the scalar state by recursion; the second batch is BUILT FROM the first's answer and the empty batch is a no-op"
  (input  (do
            (effect S (op batch (-> (List Int64) Int64)))
            (def (sum-l (: xs (List Int64)) (: i Int64) (: acc Int64))
              (match (List.at xs i)
                ((Some v) (sum-l xs (+ i 1) (+ acc v)))
                ((None u) acc)))
            (def (main (: n Int64))
              (handle S n
                ((batch (xs) s
                  (let ((s2 (+ s (sum-l xs 0 0))))
                    (resume s2 s2))))
                (let ((a (S.batch (list 1 2 3))))
                  (let ((b (S.batch (list a (+ a 1)))))
                    (let ((c (S.batch (list))))
                      (+ (* 100000 a) (+ (* 100 b) c)))))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 601919 Int64))
  (call   main (: 5 Int64)) (output (: 1103434 Int64)))
