(do
  (effect D
    (op cls (-> Int64 Int64))
    (op prod (-> Int64)))
  (def (main (: n Int64))
    (let ((piv (% n 4)))
      (handle D (tuple (: 0 Int64) (: 0 Int64) (: 0 Int64))
        ((cls (v) st
          (match st
            ((tuple a b c)
              (if (< v (+ piv 3))
                  (resume (+ 100 (+ a 1)) (tuple (+ a 1) b c))
                  (if (< (+ (+ piv 3) 4) v)
                      (resume (+ 300 (+ c 1)) (tuple a b (+ c 1)))
                      (resume (+ 200 (+ b 1)) (tuple a (+ b 1) c)))))))
         (prod () st
          (match st
            ((tuple a b c) (resume (+ (* a 100) (+ (* b 10) c)) st)))))
        (let ((p (D.cls 4))) (let ((q (D.cls 8))) (let ((r (D.cls 2))) (let ((s (D.cls 11))) (let ((t (D.cls 6))) (let ((u (D.prod)))
          (+ (* 1000 (+ (* 1000 (+ (* 1000 (+ (* 1000 (+ (* 1000 p) q)) r)) s)) t)) u)))))))))) 
  (export main))
