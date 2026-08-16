(case "tpw1 a TYPEWRITER on an eight-column carriage — each keystroke advances the column by its width, an overflow wraps to the next line answering line and leftover packed with a 9 tag, every THIRD keystroke first TABS to the next multiple of four when the seed bias is on, and the closing read packs line, column, and wrap count; the bias shifts every wrap boundary after the second keystroke"
  (input  (do
            (effect K
              (op type (-> Int64 Int64))
              (op fin (-> Int64)))
            (def (main (: n Int64))
              (handle K (tuple (: 0 Int64) (: 0 Int64) (: 0 Int64) (: 0 Int64))
                ((type (x) st
                  (match st
                    ((tuple line col w k)
                      (let ((c2 (+ (if (= (% (+ k 1) 3) 0)
                                       (if (= (% n 3) 0)
                                           col
                                           (* (+ (/ col 4) 1) 4))
                                       col)
                                   x)))
                        (if (>= c2 8)
                            (resume (+ (* (+ line 1) 100) (+ (* (- c2 8) 10) 9))
                                    (tuple (+ line 1) (- c2 8) (+ w 1) (+ k 1)))
                            (resume c2
                                    (tuple line c2 w (+ k 1))))))))
                 (fin () st
                  (match st ((tuple line col w k) (resume (+ (* line 100) (+ (* col 10) w)) st)))))
                (let ((a (K.type (: 3 Int64))))
                  (let ((b (K.type (: 5 Int64))))
                    (let ((c (K.type (: 2 Int64))))
                      (let ((d (K.type (: 6 Int64))))
                        (let ((e (K.type (: 4 Int64))))
                          (let ((f (K.fin)))
                            (+ (* 1000 (+ (* 1000 (+ (* 1000 (+ (* 1000 (+ (* 1000 a) b)) c)) d)) e)) f)))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 3109006249309303 Int64))
  (call   main (: 0 Int64)) (output (: 3109002209004242 Int64)))
