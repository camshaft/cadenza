(case "ms1 Map.swap AS the arm's state transition — the prior-value tuple half crosses dispatch, the new-map half threads"
  (input  (do
            (effect KV (op put (-> Int64 Int64 Int64)))
            (def (main (: n Int64))
              (handle KV (Map.insert Map.empty 1 n)
                ((put (k v) s
                  (match (Map.swap s k v)
                    ((tuple prior next)
                      (resume (match prior ((Some p) p) ((None _u) -9)) next)))))
                (let ((a (KV.put 1 100)))
                  (let ((b (KV.put 2 200)))
                    (let ((c (KV.put 1 111)))
                      (+ (* 10000 a) (+ (* 100 (if (= b -9) 1 0)) c)))))))
            (export main)))
  (call   main (: 7 Int64)) (output (: 70200 Int64))
  (call   main (: 0 Int64)) (output (: 200 Int64)))
