(case "nr2 the nested-record binder destructures the tuple-with-record STATE — field read and rebuilt per dispatch"
  (input  (do
            (effect S (op bump (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle S (tuple (record (= x n)) 0)
                ((bump (v) st
                  (match st
                    ((tuple (record (x a)) c)
                      (resume (+ a c) (tuple (record (= x (+ a v))) (+ c 1)))))))
                (let ((r1 (S.bump 10)))
                  (let ((r2 (S.bump 100)))
                    (+ (* 10000 r1) r2)))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 30014 Int64))
  (call   main (: 0 Int64)) (output (: 11 Int64)))
