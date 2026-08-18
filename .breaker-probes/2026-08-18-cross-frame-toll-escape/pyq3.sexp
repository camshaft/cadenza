(case "pyq3 the ESCAPING LEVY'S VALUE IS SCALED INTO THE INNER FOLD — the last inner dispatch is a thousandfold-scaled outer levy so the escaping continuation carries the scaling the inner toll and the region close while the levy's answer lands in the inner body's own arithmetic, and both frames' tolls price their own captures around the shared value"
  (input  (do
            (effect T (op levy (-> Int64)))
            (effect E (op tick (-> Int64)))
            (def (main (: n Int64))
              (handle T (% n 3)
                ((levy () t (+ (resume t (+ t 1)) (* 10000 (+ t 1)))))
                (handle E (: 5 Int64)
                  ((tick () s (+ (resume s (+ s 1)) (* 100 s))))
                  (+ (* 10 (E.tick)) (* 1000 (T.levy))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 21550 Int64))
  (call   main (: 0 Int64)) (output (: 10550 Int64)))
