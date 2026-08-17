(case "cnl1 a CANAL LOCK chamber — filling raises the water three capped at nine and REACHING nine with the gate shut auto-opens it (a seven-hundred row echoing the boat count), a boat enters only through an open gate (counted, the gate re-shutting and the chamber draining to three) else refused with the level's low digit, the read packs level boats and gate, and the seed's starting water auto-opens on the FIRST fill for one run and the SECOND for the other so the enter attempts alternate served and refused in OPPOSITE orders"
  (input  (do
            (effect C
              (op fill (-> Int64))
              (op enter (-> Int64))
              (op read (-> Int64)))
            (def (main (: n Int64))
              (handle C (tuple (+ (: 3 Int64) (* (% n 3) 3)) (: 0 Int64) (: 0 Int64))
                ((fill () st
                  (match st
                    ((tuple level gate boats)
                      (if (> (+ level 3) 9)
                          (if (= gate 0)
                              (resume (+ (: 700 Int64) boats) (tuple (: 9 Int64) (: 1 Int64) boats))
                              (resume (+ (: 90 Int64) gate) (tuple (: 9 Int64) gate boats)))
                          (if (if (= (+ level 3) 9) (= gate 0) false)
                              (resume (+ (: 700 Int64) boats) (tuple (: 9 Int64) (: 1 Int64) boats))
                              (resume (+ (* (+ level 3) 10) gate) (tuple (+ level 3) gate boats)))))))
                 (enter () st
                  (match st
                    ((tuple level gate boats)
                      (if (= gate 1)
                          (resume (+ (* (+ boats 1) 10) 3) (tuple (: 3 Int64) (: 0 Int64) (+ boats 1)))
                          (resume (+ (: 900 Int64) (% level 10)) st)))))
                 (read () st
                  (match st
                    ((tuple level gate boats)
                      (resume (+ (* level 100) (+ (* boats 10) gate)) st)))))
                (let ((a (C.fill)))
                  (let ((b (C.enter)))
                    (let ((c (C.fill)))
                      (let ((d (C.enter)))
                        (let ((f (C.read)))
                          (+ (* 10000 (+ (* 1000 (+ (* 1000 (+ (* 1000 a) b)) c)) d)) f))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 7000130609060610 Int64))
  (call   main (: 0 Int64)) (output (: 609067000130310 Int64)))
