(do
  (effect St (op next (-> Unit Int64)))
  (def (main (: n Int64))
    (handle St n
      ((next (u) s (resume s (+ s 1))))
      (match (tuple (record (a (St.next))) (St.next))
        ((tuple r y)
          (match r ((record (a x)) (+ (* 10 x) y)))))))
  (export main))
