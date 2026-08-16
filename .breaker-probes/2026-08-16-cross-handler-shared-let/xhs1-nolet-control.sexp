(do
  (effect O (op note (-> Int64 Int64)))
  (effect I (op step (-> Int64 Int64)))
  (def (main (: n Int64))
    (handle O (: 0 Int64)
      ((note (v) acc
        (resume (+ acc v) (+ acc v))))
      (handle I (: 0 Int64)
        ((step (x) col
          (resume (+ (* (+ col (+ x (% n 3))) 10) (% (O.note (+ col (+ x (% n 3)))) 10))
                  (+ col (+ x (% n 3))))))
        (let ((a (I.step (: 3 Int64))))
          (let ((b (I.step (: 5 Int64))))
            (let ((c (O.note (: 100 Int64))))
              (+ (* 1000 (+ (* 1000 a) b)) c)))))))
  (export main))
