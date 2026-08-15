(case "wtr1 the WATER-JUG puzzle over a five-cap and a three-cap jug — fill tops a jug, pour transfers the minimum of the source's content and the destination's headroom, empty drains, every answer packs both jugs as a*10+b, and the seed picks WHICH jug the routine works from so one run reaches the classic four-measure states while the other cycles"
  (input  (do
            (effect J
              (op fill (-> Int64 Int64))
              (op pour (-> Int64 Int64 Int64))
              (op emptyj (-> Int64 Int64)))
            (def (capof (: i Int64)) (if (= i 0) 5 3))
            (def (main (: n Int64))
              (handle J (tuple (: 0 Int64) (: 0 Int64))
                ((fill (i) st
                  (match st
                    ((tuple a b)
                      (if (= i 0)
                          (resume (+ 50 b) (tuple 5 b))
                          (resume (+ (* a 10) 3) (tuple a 3))))))
                 (pour (src dst) st
                  (match st
                    ((tuple a b)
                      (if (= src 0)
                          (if (< (- 3 b) a)
                              (resume (+ (* (- a (- 3 b)) 10) 3) (tuple (- a (- 3 b)) 3))
                              (resume (+ (* 0 10) (+ b a)) (tuple 0 (+ b a))))
                          (if (< (- 5 a) b)
                              (resume (+ 50 (- b (- 5 a))) (tuple 5 (- b (- 5 a))))
                              (resume (+ (* (+ a b) 10) 0) (tuple (+ a b) 0)))))))
                 (emptyj (i) st
                  (match st
                    ((tuple a b)
                      (if (= i 0)
                          (resume b (tuple 0 b))
                          (resume (* a 10) (tuple a 0)))))))
                (let ((p (J.fill (if (= (% n 3) 1) 1 0))))
                  (let ((q (J.pour (if (= (% n 3) 1) 1 0) (if (= (% n 3) 1) 0 1))))
                    (let ((r (J.fill (if (= (% n 3) 1) 1 0))))
                      (let ((s (J.pour (if (= (% n 3) 1) 1 0) (if (= (% n 3) 1) 0 1))))
                        (let ((t (J.emptyj (if (= (% n 3) 1) 0 1))))
                          (let ((u (J.pour (if (= (% n 3) 1) 1 0) (if (= (% n 3) 1) 0 1))))
                            (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 p) q)) r)) s)) t)) u)))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 33033510110 Int64))
  (call   main (: 0 Int64)) (output (: 502353535023 Int64)))
