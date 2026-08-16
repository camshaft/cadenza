(do
  (effect E (op next (-> Int64)))
  (def (main (: n Int64))
    (handle E n
      ((next () s (resume s (+ s 1))))
      (let ((r (record (a (E.next)) (b (E.next)))))
        (match r ((record (a x) (b y)) (+ (* 10 x) y))))))
  (export main))
