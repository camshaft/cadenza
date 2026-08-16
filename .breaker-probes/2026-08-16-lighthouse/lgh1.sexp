(case "lgh1 a LIGHTHOUSE sweeping four quadrants — each flash answers the lit quadrant times ten plus whether the seed-anchored ship was illuminated (advancing the rotation), log counts illuminations, and the ship's quadrant decides WHICH row carries the hit bit as the sweep wraps past it a second time on quadrant zero only"
  (input  (do
            (effect L
              (op flash (-> Int64))
              (op log (-> Int64)))
            (def (main (: n Int64))
              (handle L (tuple (: 0 Int64) (: 0 Int64))
                ((flash () st
                  (match st
                    ((tuple q seen)
                      (if (= q (% n 4))
                          (resume (+ (* q 10) 1) (tuple (% (+ q 1) 4) (+ seen 1)))
                          (resume (* q 10) (tuple (% (+ q 1) 4) seen))))))
                 (log () st
                  (match st ((tuple q seen) (resume seen st)))))
                (let ((a (L.flash)))
                  (let ((b (L.flash)))
                    (let ((c (L.flash)))
                      (let ((d (L.flash)))
                        (let ((e (L.flash)))
                          (let ((f (L.log)))
                            (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 a) b)) c)) d)) e)) f)))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 1021300001 Int64))
  (call   main (: 0 Int64)) (output (: 11020300102 Int64)))
