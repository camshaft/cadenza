(do
  (effect E (op next (-> Int64)))
  (def (main (: n Int64))
    (handle E n
      ((next () s (resume s (+ s 1))))
      (let ((r (record (a (E.next)) (b (E.next)))))
        (+ (* 10 (. r a)) (. r b)))))
  (export main))
