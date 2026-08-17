(case "sil2 the GRAIN SILO pair at three ops — a dump fills the EMPTIER silo (ties to A) capping at ten with excess SPILLED, the auger moves up to three from A into B with its own cap, the read packs both silos and spills, and the seed pre-fills silo A so every dump targets the other silo and the runs never re-align"
  (input  (do
            (effect S
              (op dump (-> Int64 Int64))
              (op auger (-> Int64))
              (op read (-> Int64)))
            (def (main (: n Int64))
              (handle S (tuple (* (% n 3) 4) (: 0 Int64) (: 0 Int64))
                ((dump (k) st
                  (match st
                    ((tuple a b sp)
                      (if (<= a b)
                          (if (> (+ a k) 10)
                              (resume (: 201 Int64) (tuple (: 10 Int64) b (+ sp (- (+ a k) 10))))
                              (resume (+ (: 100 Int64) (* (+ a k) 10)) (tuple (+ a k) b sp)))
                          (if (> (+ b k) 10)
                              (resume (: 301 Int64) (tuple a (: 10 Int64) (+ sp (- (+ b k) 10))))
                              (resume (+ (: 200 Int64) (* (+ b k) 10)) (tuple a (+ b k) sp)))))))
                 (auger () st
                  (match st
                    ((tuple a b sp)
                      (if (< a 3)
                          (if (> (+ b a) 10)
                              (resume (+ (* a 10) (% (+ sp (- (+ b a) 10)) 10)) (tuple (: 0 Int64) (: 10 Int64) (+ sp (- (+ b a) 10))))
                              (resume (+ (* a 10) (% sp 10)) (tuple (: 0 Int64) (+ b a) sp)))
                          (if (> (+ b 3) 10)
                              (resume (+ (: 30 Int64) (% (+ sp (- (+ b 3) 10)) 10)) (tuple (- a 3) (: 10 Int64) (+ sp (- (+ b 3) 10))))
                              (resume (+ (: 30 Int64) (% sp 10)) (tuple (- a 3) (+ b 3) sp)))))))
                 (read () st
                  (match st
                    ((tuple a b sp)
                      (resume (+ (* a 100) (+ (* b 10) sp)) st)))))
                (let ((p (S.dump (: 7 Int64))))
                  (let ((q (S.auger)))
                    (let ((r (S.dump (: 6 Int64))))
                      (let ((f (S.read)))
                        (+ (* 1000 (+ (* 1000 (+ (* 1000 p) q)) r)) f)))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 270030170800 Int64))
  (call   main (: 0 Int64)) (output (: 170030290490 Int64)))
