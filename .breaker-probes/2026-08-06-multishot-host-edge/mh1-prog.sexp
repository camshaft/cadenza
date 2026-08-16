(do
  (effect ask (op ask (-> Unit Int64)))
  (effect Amb (op pick (-> Unit Int64)))
  (def (main)
    (host (ask)
      (handle Amb 0
        ((pick (u) s (+ (resume 1 s) (resume 2 s))))
        (+ (* 10 (Amb.pick)) (ask.ask)))))
  (export main))
