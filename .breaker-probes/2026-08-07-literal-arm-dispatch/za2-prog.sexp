(do
  (effect St (op next (-> Unit Int64)))
  (def (main (: n Int64))
    (handle St n
      ((next (u) s (resume s (* s 2))))
      (let ((a (St.next)))
        (let ((b (St.next)))
          (match (- b a)
            (5 (+ 1000 (St.next)))
            (_o (- 0 (- b a))))))))
  (export main))
