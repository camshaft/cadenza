(case "mmx1 a LAZY-INIT extrema tracker — Option (Tuple min max) state starts None, the first feed initializes both bounds to the value, later feeds widen, range reads answer max-min with the uninitialized read answering zero"
  (input  (do
            (effect S
              (op feed (-> Int64 Int64))
              (op range (-> Int64)))
            (def (main (: n Int64))
              (handle S (: (None unit) (Option (Tuple Int64 Int64)))
                ((feed (v) st
                  (let ((p2 (match st
                              ((Some p) (match p
                                          ((tuple lo hi) (tuple (if (< v lo) v lo) (if (> v hi) v hi)))))
                              ((None u) (tuple v v)))))
                    (match p2
                      ((tuple lo2 hi2) (resume (- hi2 lo2) (Some p2))))))
                 (range () st
                  (resume (match st
                            ((Some p) (match p ((tuple lo hi) (- hi lo))))
                            ((None u) 0))
                          st)))
                (let ((a (S.range)))
                  (let ((b (S.feed n)))
                    (let ((c (S.feed 3)))
                      (let ((d (S.feed 10)))
                        (let ((e (S.range)))
                          (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 10 a) b)) c)) d)) e))))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 20707 Int64))
  (call   main (: 0 Int64)) (output (: 31010 Int64)))
