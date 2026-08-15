(case "wlkB inline-LCG twin (no binder) between walls zero and six — step draws the direction from bit one of a hidden LCG reflecting off either wall answering the landed position, whr packs position and the LCG's low digit, and the SAME direction stream walks parallel tracks until the LOW seed bounces off the floor while the high one never touches a wall"
  (input  (do
            (effect W
              (op step (-> Int64))
              (op whr (-> Int64)))
            (def (main (: n Int64))
              (handle W (tuple (+ (% n 4) 1) (: 7 Int64))
                ((step () st
                  (match st
                    ((tuple pos seed)
                      (if (= (% (>> (% (+ (* seed 5) 3) 32) 1) 2) 1)
                          (if (< 6 (+ pos 1))
                              (resume (- pos 1) (tuple (- pos 1) (% (+ (* seed 5) 3) 32)))
                              (resume (+ pos 1) (tuple (+ pos 1) (% (+ (* seed 5) 3) 32))))
                          (if (< (- pos 1) 0)
                              (resume (+ pos 1) (tuple (+ pos 1) (% (+ (* seed 5) 3) 32)))
                              (resume (- pos 1) (tuple (- pos 1) (% (+ (* seed 5) 3) 32))))))))
                 (whr () st
                  (match st
                    ((tuple pos seed) (resume (+ (* pos 10) (% seed 10)) st)))))
                (let ((a (W.step)))
                  (let ((b (W.step)))
                    (let ((c (W.step)))
                      (let ((d (W.whr)))
                        (let ((e (W.step)))
                          (let ((f (W.step)))
                            (let ((g (W.whr)))
                              (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 a) b)) c)) d)) e)) f)) g))))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 4030228030446 Int64))
  (call   main (: 0 Int64)) (output (: 2010008010226 Int64)))