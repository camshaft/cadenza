(case "oc3 a DOUBLE-wrapped Option resume value — None vs Some(None) vs Some(Some v) all distinguished by the body"
  (input  (do
            (effect O (op probe (-> Int64 (Option (Option Int64)))))
            (def (main (: n Int64))
              (handle O n
                ((probe (k) s (resume (if (< k 0)
                                          (None)
                                          (if (> k s) (Some (Some (- k s))) (Some (None))))
                                      (+ s 1))))
                (+ (match (O.probe 10)
                     ((Some inner) (match inner ((Some v) v) ((None) -1)))
                     ((None) -100))
                   (* 1000 (match (O.probe -5)
                             ((Some inner) (match inner ((Some v) v) ((None) -1)))
                             ((None) -100))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: -99995 Int64))
  (call   main (: 15 Int64)) (output (: -100001 Int64)))
