(case "lfs1 an EIGHT-BIT Fibonacci LFSR — step shifts right injecting the XOR of taps zero and two as the new high bit answering the register, peek masks the low nibble, and the two seeds fall into DIFFERENT orbits (one decays toward the injected-bit ladder, the other rings the high bits immediately)"
  (input  (do
            (effect L
              (op step (-> Int64))
              (op peek (-> Int64)))
            (def (main (: n Int64))
              (handle L (+ (% n 8) 3)
                ((step (
                  ) reg
                  (resume (| (>> reg 1) (<< (^ (& reg 1) (& (>> reg 2) 1)) 7))
                          (| (>> reg 1) (<< (^ (& reg 1) (& (>> reg 2) 1)) 7))))
                 (peek () reg (resume (& reg 15) reg)))
                (let ((a (L.step)))
                  (let ((b (L.step)))
                    (let ((c (L.peek)))
                      (let ((d (L.step)))
                        (let ((e (L.step)))
                          (let ((f (L.peek)))
                            (+ (* 1000 (+ (* 1000 (+ (* 1000 (+ (* 1000 (+ (* 1000 a) b)) c)) d)) e)) f)))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 2001001128064000 Int64))
  (call   main (: 0 Int64)) (output (: 129192000096048000 Int64)))
