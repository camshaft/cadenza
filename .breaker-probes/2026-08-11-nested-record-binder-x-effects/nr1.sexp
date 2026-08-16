(case "nr1 the NEW nested-record match-binder destructures a dispatched tuple — (tuple (record (x a)) c) in the arm, two dispatches"
  (input  (do
            (effect S (op score (-> (Tuple (Record (: x Int64)) Int64) Int64)))
            (def (main (: n Int64))
              (handle S n
                ((score (p) s
                  (match p
                    ((tuple (record (x a)) c) (resume (+ (* 100 a) (+ (* 10 c) s)) (+ s 1))))))
                (+ (S.score (tuple (record (= x 3)) 4))
                   (* 10000 (S.score (tuple (record (= x n)) 7))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 5760345 Int64))
  (call   main (: 0 Int64)) (output (: 710340 Int64)))
