(do
  (effect St (op next (-> Unit Int64)))
  (def (main (: n Int64))
    (handle St n
      ((next (u) s (resume s (+ s 1))))
      (match (record (a (St.next)) (b (St.next)))
        ((record (a x) (b y)) (+ (* 10 x) y)))))
  (export main))
