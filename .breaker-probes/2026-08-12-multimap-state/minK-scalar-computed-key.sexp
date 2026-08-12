(case "mmlminK two puts, second key COMPUTED (+ n 1), scalar map"
  (input  (do
            (effect S (op put (-> Int64 Int64 Int64)))
            (def (append-at (: m (Map Int64 Int64)) (: k Int64) (: v Int64))
              (match (Map.lookup m k)
                ((Some x) (Map.insert m k v))
                ((None u) (Map.insert m k v))))
            (def (main (: n Int64))
              (handle S Map.empty
                ((put (k v) m (let ((m2 (append-at m k v)))
                  (resume (match (Map.lookup m2 k) ((Some x) x) ((None u) 0)) m2))))
                (let ((a (S.put n n)))
                  (let ((b (S.put (+ n 1) (* 2 n))))
                    (+ (* 10 a) b)))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 36 Int64)))
