(case "quo1 a WEIGHTED QUORUM vote — each member's weight tallies ONCE (revotes no-op via the voted set), unknown members answer a sentinel, and the pass bit flips when the tally crosses the quorum"
  (input  (do
            (effect S (op vote (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle S (tuple (Map.insert (Map.insert (Map.insert Map.empty 1 3) 2 n) 3 2)
                               (tuple (Set.of (list 0)) 0))
                ((vote (k) st
                  (match st
                    ((tuple w inner)
                      (match inner
                        ((tuple voted tally)
                          (if (Set.contains voted k)
                              (resume (* tally 10) st)
                              (match (Map.lookup w k)
                                ((Some wt)
                                  (let ((t2 (+ tally wt)))
                                    (resume (+ (* t2 10) (if (>= t2 6) 1 0))
                                            (tuple w (tuple (Set.insert voted k) t2)))))
                                ((None u) (resume -1 st))))))))))
                (let ((a (S.vote 1)))
                  (let ((b (S.vote 1)))
                    (let ((c (S.vote 2)))
                      (let ((d (S.vote 9)))
                        (let ((e (S.vote 3)))
                          (+ (* 100 (+ (* 10 (+ (* 100 (+ (* 100 a) b)) c)) (+ d 2))) e))))))))
            (export main)))
  (call   main (: 4 Int64)) (output (: 303071191 Int64))
  (call   main (: 1 Int64)) (output (: 303040161 Int64)))
