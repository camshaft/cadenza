(case "fx2 float COMPARISON of two consecutive draws routes the branch — a negative seed FLIPS the order, a tail draw pins the doubling thread"
  (input  (do
            (effect E (op draw (-> Float64)))
            (def (main (: n Int64))
              (handle E (+ 1.0 (Float64.of-int n))
                ((draw () s (resume s (* s 2.0))))
                (let ((d1 (E.draw)))
                  (let ((d2 (E.draw)))
                    (+ (if (> d2 d1) (- d2 d1) (- d1 d2))
                       (E.draw))))))
            (export main)))
  (call   main (: 2 Int64)) (output (: 15.0 Float64))
  (call   main (: -5 Int64)) (output (: -12.0 Float64))
  (call   main (: 0 Int64)) (output (: 5.0 Float64)))
