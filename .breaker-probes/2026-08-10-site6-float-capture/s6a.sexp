(case "s6a wrapper binding shadows an outer binding the BODY reads"
  (input  (do
            (effect St (op get (-> Int64)))
            (def (main (: n Int64))
              (handle St n
                ((get () s (resume s (+ s 1))))
                (let ((b 10)
                      (v (let ((b 1)) (if (= b 1) (St.get) 99))))
                  (+ (* 100 v) (+ (* 10 b) (St.get))))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 404 Int64)))
