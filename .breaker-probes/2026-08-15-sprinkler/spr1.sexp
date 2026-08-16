(case "spr1 a SPRINKLER scheduler on a depleting tank — water runs a zone for the minimum of its indexed duration and the remaining tank (shortfalls accumulated), refill restores the seed-shaped capacity answering the shortfall so far, and the small tank starves two zones in the first pass and one in the second while the large tank never starves"
  (input  (do
            (effect S
              (op water (-> Int64 Int64))
              (op refill (-> Int64)))
            (def (dur (: z Int64))
              (if (= z 0) 5 (if (= z 1) 8 3)))
            (def (main (: n Int64))
              (handle S (tuple (+ 10 n) (: 0 Int64))
                ((water (z) st
                  (match st
                    ((tuple tank short)
                      (if (< tank (dur z))
                          (resume tank (tuple 0 (+ short (- (dur z) tank))))
                          (resume (dur z) (tuple (- tank (dur z)) short))))))
                 (refill () st
                  (match st
                    ((tuple tank short) (resume short (tuple (+ 10 n) short))))))
                (let ((a (S.water 0)))
                  (let ((b (S.water 1)))
                    (let ((c (S.water 2)))
                      (let ((d (S.refill)))
                        (let ((e (S.water 1)))
                          (let ((f (S.water 1)))
                            (let ((g (S.refill)))
                              (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 a) b)) c)) d)) e)) f)) g))))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 5080300080800 Int64))
  (call   main (: 0 Int64)) (output (: 5050006080212 Int64)))
