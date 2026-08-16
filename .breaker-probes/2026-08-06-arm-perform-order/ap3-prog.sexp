(do
  (effect Out (op tick (-> Unit Int64)))
  (effect In (op step (-> Unit Int64)))
  (def (main (: n Int64))
    (handle Out n
      ((tick (u) t (resume t (+ t 1))))
      (handle In 0
        ((step (u) s (resume (+ (Out.tick) (Out.tick)) s)))
        (+ (* 100 (In.step)) (In.step)))))
  (export main))
