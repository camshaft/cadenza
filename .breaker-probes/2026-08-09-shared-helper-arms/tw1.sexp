(case "tw1 arms of TWO different effects call one shared PURE helper — square-plus-one applied to each live state"
  (input  (do
            (effect P (op sq (-> Int64)))
            (effect Q (op sq (-> Int64)))
            (def (sq1 (: x Int64)) (+ (* x x) 1))
            (def (main (: n Int64))
              (handle P n
                ((sq () s (resume (sq1 s) (+ s 1))))
                (handle Q 100
                  ((sq () t (resume (sq1 t) (+ t 10))))
                  (+ (P.sq) (+ (Q.sq) (P.sq))))))
            (export main)))
  (call   main (: 2 Int64)) (output (: 10016 Int64))
  (call   main (: 0 Int64)) (output (: 10004 Int64))
  (call   main (: -3 Int64)) (output (: 10016 Int64)))
