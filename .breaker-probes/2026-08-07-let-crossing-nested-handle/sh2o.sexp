(do
  (effect A (op base (-> Unit Int64)))
  (effect B (op step (-> Unit Int64)))
  (def (main)
    (handle A 5 ((base (u) s (resume s s)))
      (handle B (A.base)
        ((step (u) t (resume t (+ t 1))))
        (+ (B.step) (B.step)))))
  (export main))
