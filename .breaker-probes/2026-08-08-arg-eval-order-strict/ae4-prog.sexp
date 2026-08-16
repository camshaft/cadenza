(do
  (effect E (op next (-> Int64)))
  (def (mix3 (: a Int64) (: b Int64) (: c Int64)) (+ (* 100 a) (+ (* 10 b) c)))
  (def (bump) (E.next))
  (def (main (: n Int64))
    (handle E n
      ((next () s (resume s (+ s 1))))
      (mix3 (E.next) (bump) (E.next))))
  (export main))
