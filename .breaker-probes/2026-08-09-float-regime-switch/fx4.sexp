(case "fx4 float sign TRICHOTOMY of a draw — positive scales, negative negates, exact 0.0 routes to the constant arm"
  (input  (do
            (effect E (op draw (-> Float64)))
            (def (main (: n Int64))
              (handle E (Float64.of-int n)
                ((draw () s (resume s (+ s 1.0))))
                (let ((d (E.draw)))
                  (+ (if (> d 0.0) (* d 10.0)
                         (if (< d 0.0) (- 0.0 d) 99.0))
                     (E.draw)))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 34.0 Float64))
  (call   main (: 0 Int64)) (output (: 100.0 Float64))
  (call   main (: -2 Int64)) (output (: 1.0 Float64)))
