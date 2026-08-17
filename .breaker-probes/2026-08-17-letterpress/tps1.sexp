(case "tps1 a LETTERPRESS galley — setting a word takes its width plus a space when the twelve-em line holds it, else BREAKS the line (counted, the word opening the next line at its bare width), justifying pads the line to twelve recording the gap as respacing, the read packs lines width and respacing, and the seed's headline stub pushes one galley's words onto broken lines while the other sets them flush with a ZERO-gap justify"
  (input  (do
            (effect T
              (op set (-> Int64 Int64))
              (op justify (-> Int64))
              (op read (-> Int64)))
            (def (main (: n Int64))
              (handle T (tuple (* (% n 3) 5) (: 0 Int64) (: 0 Int64))
                ((set (w) st
                  (match st
                    ((tuple lw lines rs)
                      (if (<= (+ lw (+ w 1)) 12)
                          (resume (+ (* (+ lw (+ w 1)) 10) (% w 10))
                                  (tuple (+ lw (+ w 1)) lines rs))
                          (resume (+ (: 700 Int64) (+ (* (+ lines 1) 10) (% w 10)))
                                  (tuple w (+ lines 1) rs))))))
                 (justify () st
                  (match st
                    ((tuple lw lines rs)
                      (resume (+ (* (- 12 lw) 10) (% (+ rs (- 12 lw)) 10))
                              (tuple (: 12 Int64) lines (+ rs (- 12 lw)))))))
                 (read () st
                  (match st
                    ((tuple lw lines rs)
                      (resume (+ (* lines 100) (+ (* lw 10) rs)) st)))))
                (let ((a (T.set (: 4 Int64))))
                  (let ((b (T.set (: 6 Int64))))
                    (let ((c (T.justify)))
                      (let ((d (T.set (: 3 Int64))))
                        (let ((f (T.read)))
                          (+ (* 10000 (+ (* 1000 (+ (* 1000 (+ (* 1000 a) b)) c)) d)) f))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 1047160667230236 Int64))
  (call   main (: 0 Int64)) (output (: 541260007130130 Int64)))
