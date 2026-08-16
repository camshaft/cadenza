(case "ns1 a MAP-OF-LISTS handler state accumulates per dispatch — two-layer path-copy across resume cycles"
  (input  (do
            (effect Db (op add (-> (Tuple String Int64) Int64)))
            (def (main (: n Int64))
              (handle Db Map.empty
                ((add (p) m
                  (match p
                    ((tuple k v)
                      (let ((xs (match (Map.lookup m k) ((Some ys) ys) ((None _u) (list)))))
                        (let ((nxs (List.push xs v)))
                          (resume (List.len nxs) (Map.insert m k nxs))))))))
                (+ (* 1000 (Db.add (tuple "a" n)))
                   (+ (* 100 (Db.add (tuple "b" 7)))
                      (+ (* 10 (Db.add (tuple "a" 6)))
                         (Db.add (tuple "a" 9)))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 1123 Int64)))
