(case "n2 the wrapped conditional's ELSE leg performs while the THEN leg is pure — inside an inner same-effect re-handle"
  (input  (do
            (effect St (op get (-> Int64)))
            (def (main (: n Int64))
              (handle St n
                ((get () s (resume s (+ s 1))))
                (let ((inner (handle St 200
                               ((get () s (resume s (+ s 7))))
                               (let ((v (let ((k (% n 2))) (if (= k 0) 55 (St.get)))))
                                 (+ (* 10 v) (St.get))))))
                  (+ (* 100 inner) (St.get)))))
            (export main)))
  (call   main (: 4 Int64)) (output (: 75004 Int64))
  (call   main (: 5 Int64)) (output (: 220705 Int64)))
