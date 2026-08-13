(case "fan1 a PUBLISH FAN-OUT — publish walks the subscriber map's enumeration and bumps EVERY value by rebuilding the whole map, a mid-run subscribe grows the fan and later publishes reach it"
  (input  (do
            (effect S
              (op sub (-> Int64 Int64))
              (op publish (-> Int64 Int64))
              (op read (-> Int64 Int64)))
            (def (bump-all (: src (List (Tuple Int64 Int64))) (: i Int64) (: v Int64) (: acc (Map Int64 Int64)))
              (match (List.at src i)
                ((Some p) (match p
                            ((tuple k old) (bump-all src (+ i 1) v (Map.insert acc k (+ old v))))))
                ((None u) acc)))
            (def (sum-vals (: src (List (Tuple Int64 Int64))) (: i Int64) (: acc Int64))
              (match (List.at src i)
                ((Some p) (match p ((tuple _k vv) (sum-vals src (+ i 1) (+ acc vv)))))
                ((None u) acc)))
            (def (main (: n Int64))
              (handle S (Map.insert (Map.insert Map.empty 1 0) 2 0)
                ((sub (k) m
                  (let ((m2 (Map.insert m k 0)))
                    (resume (Map.len m2) m2)))
                 (publish (v) m
                  (let ((m2 (bump-all (Map.to-list m) 0 v Map.empty)))
                    (resume (sum-vals (Map.to-list m2) 0 0) m2)))
                 (read (k) m
                  (resume (match (Map.lookup m k) ((Some x) x) ((None u) -1)) m)))
                (let ((a (S.publish n)))
                  (let ((b (S.sub 7)))
                    (let ((c (S.publish 2)))
                      (let ((d (S.read 1)))
                        (let ((e (S.read 7)))
                          (+ (* 10 (+ (* 100 (+ (* 100 (+ (* 10 a) b)) c)) d)) e))))))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 6312052 Int64))
  (call   main (: 0 Int64)) (output (: 306022 Int64)))
