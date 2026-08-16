(do
  (effect St (op next (-> Int64)))
  (def (main (: n Int64))
    (handle St n
      ((next () s (resume s (* s 2))))
      (do
        (def a (St.next))
        (St.next)
        (def b (St.next))
        (+ (* 100 a) (+ (* 10 b) (St.next))))))
  (export main))
