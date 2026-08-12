(case "ldd1 an EVENT-LEDGER state — a List of (tag,value) tuples appends per log dispatch, and replay folds only the MATCHING tag's values (absent tag folds to zero)"
  (input  (do
            (effect S
              (op log (-> Int64 Int64 Int64))
              (op replay (-> Int64 Int64)))
            (def (fold-tag (: xs (List (Tuple Int64 Int64))) (: t Int64) (: i Int64) (: acc Int64))
              (match (List.at xs i)
                ((Some e) (match e
                            ((tuple tt v) (fold-tag xs t (+ i 1) (if (= tt t) (+ acc v) acc)))))
                ((None u) acc)))
            (def (main (: n Int64))
              (handle S (: (list) (List (Tuple Int64 Int64)))
                ((log (t v) led
                  (let ((led2 (List.push led (tuple t v))))
                    (resume (List.len led2) led2)))
                 (replay (t) led (resume (fold-tag led t 0 0) led)))
                (let ((a (S.log 1 n)))
                  (let ((b (S.log 2 7)))
                    (let ((c (S.log 1 (+ n 2))))
                      (let ((d (S.replay 1)))
                        (let ((e (S.replay 2)))
                          (let ((f (S.replay 9)))
                            (+ (* 10 (+ (* 100 (+ (* 1000 (+ (* 10 (+ (* 10 a) b)) c)) d)) e)) f)))))))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 123008070 Int64))
  (call   main (: 20 Int64)) (output (: 123042070 Int64)))
