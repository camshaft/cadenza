(do
  (effect E (op next (-> Int64)))
  (def (main (: n Int64))
    (handle E n
      ((next () s (resume s (+ s 1))))
      (match (record (a (E.next)) (b (E.next)))
        ((record (a x) (b y)) (+ (* 10 x) y)))))
  (export main))
