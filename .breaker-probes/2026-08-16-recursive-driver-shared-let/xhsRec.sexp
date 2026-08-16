(case "xhsRec recursive-DRIVER + shared-let + mid-arm foreign perform — drive recursively performs I.tick each iter; the tick arm let-binds c2, performs O.note(c2) mid-arm, resumes packing nv, threads c2. Collapse excluded by in_recursive_specialize -> distribute; does the shared binder diverge like xhsGrow?"
  (input
    (do
      (effect O (op note (-> Int64 Int64)))
      (effect I (op tick (-> Int64 Int64)))
      (def (drive (: k Int64))
        (if (<= k 0) 0 (+ (I.tick k) (drive (- k 1)))))
      (def (main (: n Int64))
        (handle O (: 0 Int64)
          ((note (v) acc (resume (+ acc v) (+ acc v))))
          (handle I (: 0 Int64)
            ((tick (x) col
              (let ((c2 (+ col (+ x (% n 3)))))
                (let ((nv (O.note c2)))
                  (resume (+ (* c2 10) (% nv 10)) c2)))))
            (drive n))))
      (export main)))
  (call main (: 3 Int64)) (output (: 0 Int64)))
