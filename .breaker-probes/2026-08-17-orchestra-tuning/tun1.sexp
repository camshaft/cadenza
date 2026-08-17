(case "tun1 an ORCHESTRA tuning round — each section plays at the stand pitch plus its offset and the podium compares against forty-two, within one counts IN TUNE (answer packs the sounded note), otherwise a RETUNE shifts the stand HALFWAY toward the gap (signed integer halving) counting itself in an eight-hundred row with the gap's magnitude, the read packs pitch tuned and retunes, and the seed's stand pitch makes the FIRST section retune on one round and play in tune on the other with the drift echoing through every later gap"
  (input  (do
            (effect O
              (op play (-> Int64 Int64))
              (op read (-> Int64)))
            (def (main (: n Int64))
              (handle O (tuple (+ (: 40 Int64) (% n 3)) (: 0 Int64) (: 0 Int64))
                ((play (off) st
                  (match st
                    ((tuple pitch tuned ret)
                      (if (if (<= (- 42 (+ pitch off)) 1) (>= (- 42 (+ pitch off)) -1) false)
                          (resume (+ (* (+ pitch off) 10) 1)
                                  (tuple pitch (+ tuned 1) ret))
                          (resume (+ (: 800 Int64)
                                     (+ (* (if (> (- 42 (+ pitch off)) 0)
                                              (- 42 (+ pitch off))
                                              (- 0 (- 42 (+ pitch off)))) 10)
                                        (+ ret 1)))
                                  (tuple (+ pitch (/ (- 42 (+ pitch off)) 2)) tuned (+ ret 1)))))))
                 (read () st
                  (match st
                    ((tuple pitch tuned ret)
                      (resume (+ (* pitch 100) (+ (* tuned 10) ret)) st)))))
                (let ((a (O.play (: 0 Int64))))
                  (let ((b (O.play (: 3 Int64))))
                    (let ((c (O.play (: -1 Int64))))
                      (let ((d (O.play (: 1 Int64))))
                        (let ((f (O.read)))
                          (+ (* 10000 (+ (* 1000 (+ (* 1000 (+ (* 1000 a) b)) c)) d)) f))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 4118218324214122 Int64))
  (call   main (: 0 Int64)) (output (: 8218228334214113 Int64)))
