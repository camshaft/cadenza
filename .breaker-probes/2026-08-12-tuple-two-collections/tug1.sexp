(case "tug1 a TUPLE state pairing TWO collections — a List and a Map advance through different ops, and a cross op reads BOTH halves in one answer"
  (input  (do
            (effect S
              (op pushl (-> Int64 Int64))
              (op putm (-> Int64 Int64 Int64))
              (op cross (-> Int64)))
            (def (main (: n Int64))
              (handle S (tuple (: (list) (List Int64)) Map.empty)
                ((pushl (v) st
                  (match st
                    ((tuple xs m) (let ((xs2 (List.push xs v)))
                      (resume (List.len xs2) (tuple xs2 m))))))
                 (putm (k v) st
                  (match st
                    ((tuple xs m) (let ((m2 (Map.insert m k v)))
                      (resume (Map.len m2) (tuple xs m2))))))
                 (cross () st
                  (match st
                    ((tuple xs m)
                      (resume (+ (* 10 (match (List.at xs 0)
                                         ((Some h) (match (Map.lookup m h) ((Some x) x) ((None u) 0)))
                                         ((None u) -1)))
                                 (List.len xs))
                              st)))))
                (let ((a (S.pushl n)))
                  (let ((b (S.putm n (* 2 n))))
                    (let ((c (S.pushl (+ n 1))))
                      (let ((d (S.cross)))
                        (+ (* 1000 (+ (* 10 (+ (* 10 a) b)) c)) d)))))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 112062 Int64))
  (call   main (: 0 Int64)) (output (: 112002 Int64)))
