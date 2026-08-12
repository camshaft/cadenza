(case "rmp1 sibling: RECORD state wrapping a Map plus a counter — computed perform keys; the arm answers 10*lookup + the ADVANCED counter so both fields are observed"
  (input  (do
            (effect S (op put (-> Int64 Int64 Int64)))
            (def (main (: n Int64))
              (handle S (record (= m Map.empty) (= cnt 0))
                ((put (k v) st
                  (let ((m2 (Map.insert (. st m) k v)))
                    (let ((c2 (+ (. st cnt) 1)))
                      (resume (+ (* 10 (match (Map.lookup m2 k) ((Some x) x) ((None u) 0))) c2)
                              (record (= m m2) (= cnt c2)))))))
                (let ((a (S.put (+ n 1) n)))
                  (let ((b (S.put (* 2 n) (+ n 5))))
                    (+ (* 100 a) b)))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 3182 Int64))
  (call   main (: 9 Int64)) (output (: 9242 Int64)))
