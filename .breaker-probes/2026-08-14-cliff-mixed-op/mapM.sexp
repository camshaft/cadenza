(case "mapM a MAP-valued dual-use let in a mixed-op region — put binds the inserted map once for both slots, total sums the values between puts"
  (input  (do
            (effect M
              (op put (-> Int64 Int64))
              (op total (-> Int64)))
            (def (sumv (: rows (List (Tuple Int64 Int64))) (: i Int64) (: acc Int64))
              (if (< i (List.len rows))
                  (sumv rows (+ i 1) (+ acc (match (List.at rows i) ((Some (tuple k v)) v) ((None u) 0))))
                  acc))
            (def (main (: n Int64))
              (handle M (: (map) (Map Int64 Int64))
                ((put (k) m
                  (let ((m2 (Map.insert m k (* k 2))))
                    (resume (Map.len m2) m2)))
                 (total () m (resume (sumv (Map.to-list m) 0 0) m)))
                (let ((a (M.put (+ n 1))))
                  (let ((b (M.total)))
                    (let ((c (M.put 7)))
                      (let ((d (M.total)))
                        (+ (* 100 (+ (* 100 (+ (* 100 a) b)) c)) d)))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 1220236 Int64))
  (call   main (: 0 Int64)) (output (: 1020216 Int64)))
