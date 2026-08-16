(do
  (effect K (op type (-> Int64 Int64)) (op fin (-> Int64)))
  (def (main (: n Int64))
    (handle K (tuple (: 0 Int64) (: 0 Int64) (: 0 Int64) (: 0 Int64))
      ((type (x) st
        (match st
          ((tuple line col w k)
            (let ((c2 (+ (if (= (% (+ k 1) 3) 0)
                             (if (= (% n 3) 0) col (* (+ (/ col 4) 1) 4))
                             col)
                         x)))
              (if (>= c2 8)
                  (resume (+ (* (+ line 1) 100) (+ (* (- c2 8) 10) 9))
                          (tuple (+ line 1) (- c2 8) (+ w 1) (+ k 1)))
                  (resume c2 (tuple line c2 w (+ k 1))))))))
       (fin () st
        (resume (: 7 Int64) st)))
      (let ((a (K.type (: 3 Int64))))
        (let ((b (K.type (: 5 Int64))))
          (let ((c (K.type (: 2 Int64))))
            (let ((f (K.fin)))
              (+ (* 1000 (+ (* 100 (+ (* 10 a) b)) c)) f)))))))
  (export main))
