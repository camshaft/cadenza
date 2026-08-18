(do
  (effect E (op tick (-> Int64 Int64)))
  (def (main (: n Int64))
    (handle E n
      ((tick (v) s (+ (resume v s) v)))
      (let ((a (E.tick 3)))
        (let ((b (+ a 1)))
          (E.tick b)))))
  (export main))
