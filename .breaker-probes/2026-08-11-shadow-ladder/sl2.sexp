(case "sl2 the OUTER state is untouched by a closed shadow — draws before and after the inner region continue the outer stride"
  (input  (do
            (effect St (op get (-> Int64)))
            (def (main (: n Int64))
              (handle St n
                ((get () s (resume s (+ s 1))))
                (let ((inner (handle St 100
                               ((get () t (resume t (+ t 10))))
                               (+ (St.get) (St.get)))))
                  (+ (* 1000 inner) (+ (* 10 (St.get)) (St.get))))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 210034 Int64))
  (call   main (: 0 Int64)) (output (: 210001 Int64)))
