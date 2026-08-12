(case "hst1 a HISTOGRAM state with in-arm BUCKETING — observe divides into decade buckets and counts, mode walks the sorted enumeration to answer the densest bucket"
  (input  (do
            (effect S
              (op obs (-> Int64 Int64))
              (op mode (-> Int64)))
            (def (best (: xs (List (Tuple Int64 Int64))) (: i Int64) (: bk Int64) (: bc Int64))
              (match (List.at xs i)
                ((Some e) (match e
                            ((tuple k c) (if (> c bc) (best xs (+ i 1) k c) (best xs (+ i 1) bk bc)))))
                ((None u) (+ (* 10 bk) bc))))
            (def (main (: n Int64))
              (handle S (: Map.empty (Map Int64 Int64))
                ((obs (v) m
                  (let ((b (/ v 10)))
                    (let ((c2 (match (Map.lookup m b) ((Some c) (+ c 1)) ((None u) 1))))
                      (resume c2 (Map.insert m b c2)))))
                 (mode () m
                  (resume (if (= (Map.len m) 0) -1 (best (Map.to-list m) 0 -1 -1)) m)))
                (let ((a (S.obs n)))
                  (let ((b (S.obs (+ n 1))))
                    (let ((c (S.obs 35)))
                      (let ((d (S.obs (+ n 2))))
                        (let ((e (S.mode)))
                          (+ (* 1000 (+ (* 10 (+ (* 10 (+ (* 10 a) b)) c)) d)) e))))))))
            (export main)))
  (call   main (: 12 Int64)) (output (: 1213013 Int64))
  (call   main (: 33 Int64)) (output (: 1234034 Int64)))
