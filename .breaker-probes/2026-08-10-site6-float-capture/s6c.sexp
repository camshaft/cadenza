(case "s6c TWO block-wrapped branch-performing inits in one let, the second's wrapper reads the first's value"
  (input  (do
            (effect St (op get (-> Int64)))
            (def (main (: n Int64))
              (handle St n
                ((get () s (resume s (+ s 1))))
                (let ((v (let ((a 1)) (if (= a 1) (St.get) 90)))
                      (w (let ((c (* 2 v))) (if (= c 6) (St.get) 80))))
                  (+ (* 100 v) (+ (* 10 w) (St.get))))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 345 Int64))
  (call   main (: 7 Int64)) (output (: 1508 Int64)))
