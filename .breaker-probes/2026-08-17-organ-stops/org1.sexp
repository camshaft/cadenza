(case "org1 an ORGAN with three ranks and a stop selector — chord passes THREE pipe volumes and the state's selector picks WHICH argument sounds through a nested-if ladder folding it into the played total and stepping the selector two ranks around, swell re-aims the selector by the played total leaving the total untouched, and the seed sets the opening stop so the two runs sound different ranks from the same three-volume chords at every dispatch"
  (input  (do
            (effect L
              (op chord (-> Int64 Int64 Int64 Int64))
              (op swell (-> Int64)))
            (def (main (: n Int64))
              (handle L (tuple (% n 3) (: 0 Int64))
                ((chord (a b c) st
                  (match st
                    ((tuple sel played)
                      (let ((v (if (= sel 0) a (if (= sel 1) b c))))
                        (resume (+ (* (+ sel 1) 100) (+ (* v 10) (% (+ played v) 10)))
                                (tuple (% (+ sel 2) 3) (+ played v)))))))
                 (swell () st
                  (match st
                    ((tuple sel played)
                      (resume (+ (* (% (+ sel played) 3) 10) (% played 10))
                              (tuple (% (+ sel played) 3) played))))))
                (let ((a (L.chord 4 7 2)))
                  (let ((b (L.chord 5 1 8)))
                    (let ((c (L.swell)))
                      (let ((d (L.chord 3 6 9)))
                        (let ((e (L.chord 2 2 2)))
                          (let ((f (L.swell)))
                            (+ (* 1000 (+ (* 1000 (+ (* 1000 (+ (* 1000 (+ (* 1000 a) b)) c)) d)) e)) f)))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 277152022391223023 Int64))
  (call   main (: 0 Int64)) (output (: 144382012268120010 Int64)))
