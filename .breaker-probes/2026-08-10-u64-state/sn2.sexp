(case "sn2 TWO whole-state snapshots straddle interleaved bumps — the first snapshot is IMMUTABLE, the pair differ by exactly the bumps between them"
  (input  (do
            (effect E (op bumpa (-> Int64)) (op bumpb (-> Int64)) (op snap (-> (Tuple Int64 Int64))))
            (def (main (: n Int64))
              (handle E (tuple n 10)
                ((bumpa () st (match st ((tuple a b) (resume a (tuple (+ a 1) b)))))
                 (bumpb () st (match st ((tuple a b) (resume b (tuple a (+ b 5))))))
                 (snap () st (resume st st)))
                (do (E.bumpa)
                    (let ((s1 (E.snap)))
                      (do (E.bumpb) (E.bumpa)
                          (let ((s2 (E.snap)))
                            (match s1
                              ((tuple a1 b1)
                               (match s2
                                 ((tuple a2 b2)
                                  (+ (+ a1 b1) (* 10 (+ a2 b2)))))))))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 236 Int64))
  (call   main (: 0 Int64)) (output (: 181 Int64))
  (call   main (: -8 Int64)) (output (: 93 Int64)))
