(case "nl2 the nested-record binder in LIST-ELEMENT pattern position — the head record's field binds, the rest carries full records"
  (input  (do
            (def (main (: n Int64))
              (let ((xs (list (record (= x n) (= y 2)) (record (= x 7) (= y 8)))))
                (match xs
                  ((list (record (= x a)) .. rest)
                    (+ (* 1000 a)
                       (+ (* 10 (List.len rest))
                          (match (List.at rest 0)
                            ((Some r2) (. r2 y))
                            ((None _u) -1)))))
                  (_other -9))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 3018 Int64))
  (call   main (: 0 Int64)) (output (: 18 Int64)))
