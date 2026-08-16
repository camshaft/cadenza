(do
  (effect Db (op add (-> Int64 Int64)))
  (def (main (: n Int64))
    (handle Db (record (seen (Set.of (list))) (total 0))
      ((add (v) st
        (let ((ns (Set.insert (. st seen) v)))
          (resume (Set.len ns)
                  (record (seen ns) (total (+ (. st total) v)))))))
      (+ (* 100 (Db.add n))
         (+ (* 10 (Db.add n))
            (Db.add 7)))))
  (export main))
