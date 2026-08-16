(case "gs6 a TWO-VARIANT generic (Either a b) by name — parity routes construction between Left and Right, the annotated consumer matches both"
  (input  (do
            (type (Either a b) (Left a) (Right b))
            (def (score (: e (Either Int64 Int64)))
              (match e
                ((Left x) (* 10 x))
                ((Right y) y)))
            (def (main (: k Int64))
              (score (if (= (% k 2) 0) (Left k) (Right (+ k 1)))))
            (export main)))
  (call   main (: 4 Int64)) (output (: 40 Int64))
  (call   main (: 3 Int64)) (output (: 4 Int64))
  (call   main (: -2 Int64)) (output (: -20 Int64)))
