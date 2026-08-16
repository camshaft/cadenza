(case "n3 DECLINE-WITNESS wrapper init is a DISCHARGED inner handle — reaches_any_perform over-reports, Site-6 peel declines (honest floor)"
  (input  (do
            (effect St (op get (-> Int64)))
            (def (main (: n Int64))
              (handle St 30
                ((get () s (resume s (+ s 1))))
                (let ((v (let ((w (handle St 30
                                    ((get () s (resume s (+ s 2))))
                                    (+ (St.get) (St.get)))))
                           (if (= (+ w n) 65) (St.get) w))))
                  (+ (* 10 v) (St.get)))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 34 Int64))
  (call   main (: 9 Int64)) (output (: 630 Int64)))
