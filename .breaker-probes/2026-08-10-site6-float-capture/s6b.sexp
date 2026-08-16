(case "s6b wrapper binding shadows an outer binding a LATER outer init reads"
  (input  (do
            (effect St (op get (-> Int64)))
            (def (main (: n Int64))
              (handle St n
                ((get () s (resume s (+ s 1))))
                (let ((b 10)
                      (v (let ((b 1)) (if (= b 1) (St.get) 99)))
                      (w (* 1000 b)))
                  (+ w (+ (* 100 v) (St.get))))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 10304 Int64)))
