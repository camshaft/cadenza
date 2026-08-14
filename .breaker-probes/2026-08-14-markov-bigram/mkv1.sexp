(case "mkv1 a MARKOV bigram counter — each feed keys the map by the (previous, current) PAIR built from the state and the argument, so the same value arriving after different predecessors lands in different buckets"
  (input  (do
            (effect S (op feed (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle S (tuple 0 (: Map.empty (Map (Tuple Int64 Int64) Int64)))
                ((feed (v) st
                  (match st
                    ((tuple prev m)
                      (let ((k (tuple prev v)))
                        (let ((c2 (+ (match (Map.lookup m k) ((Some c) c) ((None u) 0)) 1)))
                          (resume c2 (tuple v (Map.insert m k c2)))))))))
                (let ((a (S.feed n)))
                  (let ((b (S.feed 2)))
                    (let ((c (S.feed n)))
                      (let ((d (S.feed 2)))
                        (let ((e (S.feed n)))
                          (+ (* 10 (+ (* 10 (+ (* 10 (+ (* 10 a) b)) c)) d)) e))))))))
            (export main)))
  (call   main (: 2 Int64)) (output (: 11234 Int64))
  (call   main (: 5 Int64)) (output (: 11122 Int64)))
