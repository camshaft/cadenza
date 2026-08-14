(case "dstD five cls dispatches, pivots in state not referencing n in the arm"
  (input  (do
            (effect D (op cls (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle D (tuple (: 0 Int64) (: 0 Int64) (: 0 Int64) (+ (% n 4) 3) (+ (+ (% n 4) 3) 4))
                ((cls (v) st
                  (match st
                    ((tuple a b c lo hi)
                      (if (< v lo)
                          (resume (+ 100 (+ a 1)) (tuple (+ a 1) b c lo hi))
                          (if (< hi v)
                              (resume (+ 300 (+ c 1)) (tuple a b (+ c 1) lo hi))
                              (resume (+ 200 (+ b 1)) (tuple a (+ b 1) c lo hi))))))))
                (let ((p (D.cls 4)))
                  (let ((q (D.cls 8)))
                    (let ((r (D.cls 2)))
                      (let ((s (D.cls 11)))
                        (let ((t (D.cls 6)))
                          (+ (* 1000 (+ (* 1000 (+ (* 1000 (+ (* 1000 p) q)) r)) s)) t))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 101201102301202 Int64))
  (call   main (: 0 Int64)) (output (: 201301101302202 Int64)))