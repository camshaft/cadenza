(effect S (op add (-> Int64 Int64)))
(def (main (: n Int64))
  (handle S 0
    ((add (v) s
      (resume (% (+ s v) 10) (+ s v))))
    (let ((a (S.add 3))) (let ((b (S.add 4))) (let ((c (S.add 9))) (let ((d (S.add 1))) (let ((e (S.add 6))) (let ((f (S.add 2))) (let ((g (S.add 8))) (+ (* 1000000 a) (+ (* 100000 b) (+ (* 10000 c) (+ (* 1000 d) (+ (* 100 e) (+ (* 10 f) (* 1 g))))))))))))))))
(export main)