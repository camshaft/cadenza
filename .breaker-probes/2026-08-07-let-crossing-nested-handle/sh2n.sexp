(do
  (effect A (op base (-> Int64)))
  (effect B (op step (-> Int64)))
  (def (main (: n Int64))
    (handle A n
      ((base () s (resume s s)))
      (let ((seed (A.base)))
        (handle B seed
          ((step () t (resume t (+ t 1))))
          (+ (B.step) (B.step))))))
  (export main))
