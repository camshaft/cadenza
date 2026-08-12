(case "xh1 cross-handler: the INNER arm performs the OUTER op with a COMPUTED key, and the outer arm is the two-lookup-match Map shape — the #21 partition across a handler boundary"
  (input  (do
            (effect T (op put (-> Int64 Int64 Int64)))
            (effect S (op bump (-> Int64)))
            (def (main (: n Int64))
              (handle T Map.empty
                ((put (k v) m
                  (let ((m2 (match (Map.lookup m k)
                              ((Some x) (Map.insert m k v))
                              ((None u) (Map.insert m k v)))))
                    (resume (match (Map.lookup m2 k) ((Some x) x) ((None u) 0)) m2))))
                (handle S n
                  ((bump () s
                    (let ((t (T.put (+ s 1) s)))
                      (resume (+ (* 10 t) s) (+ s t)))))
                  (let ((a (S.bump)))
                    (let ((b (S.bump)))
                      (+ (* 100 a) b))))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 3366 Int64))
  (call   main (: 7 Int64)) (output (: 7854 Int64)))
