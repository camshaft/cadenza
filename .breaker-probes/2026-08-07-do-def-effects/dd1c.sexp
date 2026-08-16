(do
  (effect St (op next (-> Int64)))
  (def (main (: n Int64))
    (handle St n
      ((next () s (resume s (* s 2))))
      (do
        (St.next)
        (def a (St.next))
        (+ (* 100 a) (St.next)))))
  (export main))
