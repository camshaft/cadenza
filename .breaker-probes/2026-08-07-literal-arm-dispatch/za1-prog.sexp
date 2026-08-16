(do
  (effect St (op next (-> Unit Int64)))
  (def (main (: n Int64))
    (handle St n
      ((next (u) s (resume s (+ s 1))))
      (let ((k (St.next)))
        (match k
          (5 (+ 100 (St.next)))
          (6 200)
          (_o 300)))))
  (export main))
