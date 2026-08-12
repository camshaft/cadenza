(case "olc1 an Option-of-LIST handler state lifecycle — None is uninitialized, push initializes-or-appends, take scores and RESETS to None, a later push re-initializes"
  (input  (do
            (effect S
              (op push (-> Int64 Int64))
              (op take (-> Int64)))
            (def (main (: n Int64))
              (handle S (: (None unit) (Option (List Int64)))
                ((push (v) st
                  (let ((xs2 (match st
                               ((Some xs) (List.push xs v))
                               ((None u) (list v)))))
                    (resume (List.len xs2) (Some xs2))))
                 (take () st
                  (resume (match st
                            ((Some xs) (+ (* 10 (List.len xs))
                                          (match (List.at xs 0) ((Some h) h) ((None u) 0))))
                            ((None u) 0))
                          (: (None unit) (Option (List Int64))))))
                (let ((a (S.push n)))
                  (let ((b (S.push (+ n 1))))
                    (let ((c (S.take)))
                      (let ((d (S.push (+ n 2))))
                        (let ((e (S.take)))
                          (+ (* 100 (+ (* 10 (+ (* 100 (+ (* 10 a) b)) c)) d)) e))))))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 1223115 Int64))
  (call   main (: 8 Int64)) (output (: 1228120 Int64)))
