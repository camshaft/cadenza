(effect S (op add (-> Int64 Int64)))
(def (main (: n Int64))
  (handle S (list 0)
    ((add (v) pre
      (let ((t (+ (match (List.at pre (- (List.len pre) 1)) ((Some x) x) ((None _u) 0)) v)))
        (resume (% t 10) (List.push pre t)))))
    (let ((a (S.add 3))) (let ((b (S.add 4))) (let ((c (S.add 9))) (let ((d (S.add 1))) (+ (* 1000 a) (+ (* 100 b) (+ (* 10 c) (* 1 d))))))))))
(export main)