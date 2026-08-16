(do
  (effect E (op next (-> Int64)))
  (def (fold (: k Int64) (: acc Int64))
    (if (<= k 0) acc (fold (- k 1) (+ acc (E.next)))))
  (def (main (: n Int64))
    (handle E n
      ((next () s (resume s (+ s 1))))
      (fold 3 0)))
  (export main))
