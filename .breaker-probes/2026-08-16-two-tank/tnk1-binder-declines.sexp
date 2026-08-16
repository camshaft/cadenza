(case "tnk1 a TWO-TANK siphon driven by the level difference — each siphon binds a quarter of the always-positive gap moving it A-to-B answering the transfer, pour tops tank A, levels reads the residual gap, and the seed sets tank B so the same siphon sequence converges through DIFFERENT geometric ladders to the same final gap (A stays the fuller tank throughout)"
  (input  (do
            (effect T
              (op siphon (-> Int64))
              (op pour (-> Int64 Int64))
              (op levels (-> Int64)))
            (def (main (: n Int64))
              (handle T (tuple (: 40 Int64) (* n 2))
                ((siphon () st
                  (match st
                    ((tuple a b)
                      (match (/ (- a b) 4)
                        (d (resume d (tuple (- a d) (+ b d))))))))
                 (pour (v) st
                  (match st
                    ((tuple a b) (resume (+ a v) (tuple (+ a v) b)))))
                 (levels () st
                  (match st ((tuple a b) (resume (- a b) st)))))
                (let ((p (T.siphon)))
                  (let ((q (T.siphon)))
                    (let ((r (T.pour 12)))
                      (let ((s (T.siphon)))
                        (let ((t (T.siphon)))
                          (let ((u (T.siphon)))
                            (let ((v (T.levels)))
                              (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 p) q)) r)) s)) t)) u)) v))))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 5024504020104 Int64))
  (call   main (: 0 Int64)) (output (: 10053705030104 Int64)))
