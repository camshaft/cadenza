(do
  (effect St (op next (-> Unit Int64)))
  (def (main (: n Int64))
    (handle St n
      ((next (u) s (resume s (+ s 1))))
      (let ((v (St.next)))
        (let ((xs (list v v)))
          (let ((nested (list xs xs)))
            (+ (* 100 (List.len nested))
               (match (List.at nested 1)
                 ((Some inner) (match (List.at inner 0) ((Some x) x) ((None _u) -1)))
                 ((None _u) -1))))))))
  (export main))
