(do
  (effect St (op next (-> Unit Int64)))
  (def (main (: n Int64))
    (handle St n
      ((next (u) s (resume s (+ s 1))))
      (match (Some (St.next))
        ((Some x) (* x 10))
        ((None _u) -1))))
  (export main))
