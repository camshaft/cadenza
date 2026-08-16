(case "tt1 a NESTED-tuple op ARG destructured two levels inside the arm — both dispatches read the live state"
  (input  (do
            (effect E (op deep (-> (Tuple (Tuple Int64 Int64) Int64) Int64)))
            (def (main (: n Int64))
              (handle E n
                ((deep (p) s (match p
                               ((tuple inner c) (match inner
                                                  ((tuple a b) (resume (+ (* 100 a) (+ (* 10 b) (+ c s)))
                                                                       (+ s 1))))))))
                (+ (E.deep (tuple (tuple 1 2) 3)) (E.deep (tuple (tuple 4 5) 6)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 590 Int64))
  (call   main (: 0 Int64)) (output (: 580 Int64)))
