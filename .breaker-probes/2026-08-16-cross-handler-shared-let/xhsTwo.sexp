(case "xhsTwo two DISTINCT outer handlers — inner step performs O.note(c2) then P.tick(c2), both foreign, both with the shared binder; tests #fa freeze for two handlers at different drain levels"
  (input
    (do
      (effect P (op tick (-> Int64 Int64)))
      (effect O (op note (-> Int64 Int64)))
      (effect I (op step (-> Int64 Int64)))
      (def (main (: n Int64))
        (handle P (: 0 Int64)
          ((tick (w) pa (resume (+ pa w) (+ pa w))))
          (handle O (: 0 Int64)
            ((note (v) oa (resume (+ oa v) (+ oa v))))
            (handle I (: 0 Int64)
              ((step (x) col
                (let ((c2 (+ col (+ x (% n 3)))))
                  (let ((nv (O.note c2)))
                    (let ((tw (P.tick c2)))
                      (resume (+ (* c2 100) (+ (* (% nv 10) 10) (% tw 10))) c2))))))
              (let ((a (I.step (: 3 Int64))))
                (let ((b (I.step (: 5 Int64))))
                  (+ (* 1000 (+ (* 1000 a) b)) (+ (O.note (: 100 Int64)) (P.tick (: 100 Int64))))))))))
      (export main)))
  (call main (: 10 Int64)) (output (: 445044228 Int64))
  (call main (: 0 Int64)) (output (: 333811222 Int64)))
