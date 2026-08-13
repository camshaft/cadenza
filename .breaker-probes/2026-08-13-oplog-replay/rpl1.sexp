(case "rpl1 an OP-LOG REPLAY state — apply advances the value and logs its delta, replay re-applies the WHOLE log to the current value keeping the log intact, so a second replay after more logging compounds"
  (input  (do
            (effect S
              (op apply (-> Int64 Int64))
              (op replay (-> Int64)))
            (def (sum-log (: ds (List Int64)) (: i Int64) (: acc Int64))
              (match (List.at ds i)
                ((Some d) (sum-log ds (+ i 1) (+ acc d)))
                ((None u) acc)))
            (def (main (: n Int64))
              (handle S (tuple n (: (list) (List Int64)))
                ((apply (d) st
                  (match st
                    ((tuple v log)
                      (resume (+ v d) (tuple (+ v d) (List.push log d))))))
                 (replay () st
                  (match st
                    ((tuple v log)
                      (let ((v2 (+ v (sum-log log 0 0))))
                        (resume v2 (tuple v2 log)))))))
                (let ((a (S.apply 3)))
                  (let ((b (S.apply 4)))
                    (let ((c (S.replay)))
                      (let ((d (S.apply 1)))
                        (let ((e (S.replay)))
                          (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 a) b)) c)) d)) e))))))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 307141523 Int64))
  (call   main (: 5 Int64)) (output (: 812192028 Int64)))
