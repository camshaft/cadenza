(case "rok1 a ROOK sliding a bitboard rank — a recursive scan walks square by square until the wall or a blocker, a blocker square is CAPTURED (bit cleared, file summed) with a 9 tag, a wall stop answers the square plain, the read packs captured-file sum position and count, and the seed places one blocker near or far so the same four slides capture different files in a different order"
  (input  (do
            (effect B
              (op slide (-> Int64 Int64))
              (op read (-> Int64)))
            (def (scan (: p Int64) (: d Int64) (: occ Int64))
              (if (if (< (+ p d) 0) true (> (+ p d) 7))
                  p
                  (if (= (& (>> occ (+ p d)) 1) 1)
                      (- 0 (+ (+ p d) 1))
                      (scan (+ p d) d occ))))
            (def (main (: n Int64))
              (handle B (tuple (: 0 Int64) (| (<< (: 1 Int64) (+ 1 (* (% n 3) 5))) (<< (: 1 Int64) 3)) (: 0 Int64) (: 0 Int64))
                ((slide (d) st
                  (match st
                    ((tuple pos occ caps capsum)
                      (match (scan pos d occ)
                        (r (if (< r 0)
                               (resume (+ (* (- (- 0 r) 1) 10) 9)
                                       (tuple (- (- 0 r) 1)
                                              (^ occ (<< (: 1 Int64) (- (- 0 r) 1)))
                                              (+ caps 1)
                                              (+ capsum (- (- 0 r) 1))))
                               (resume (* r 10) (tuple r occ caps capsum))))))))
                 (read () st
                  (match st
                    ((tuple pos occ caps capsum)
                      (resume (+ (* capsum 100) (+ (* pos 10) caps)) st)))))
                (let ((a (B.slide (: 1 Int64))))
                  (let ((b (B.slide (: -1 Int64))))
                    (let ((c (B.slide (: 1 Int64))))
                      (let ((d (B.slide (: -1 Int64))))
                        (let ((f (B.read)))
                          (+ (* 1000 (+ (* 1000 (+ (* 1000 (+ (* 1000 a) b)) c)) d)) f))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 39000069000902 Int64))
  (call   main (: 0 Int64)) (output (: 19000039000402 Int64)))
