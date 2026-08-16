(case "s6e nested TWO-deep pure wrappers where the inner reads the outer, conditional reads both"
  (input  (do
            (effect St (op get (-> Int64)))
            (def (main (: n Int64))
              (handle St n
                ((get () s (resume s (+ s 1))))
                (let ((v (let ((a (* 2 n)))
                           (let ((b (+ a 1)))
                             (if (= b 7) (St.get) (+ a b))))))
                  (+ (* 10 v) (St.get)))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 34 Int64))
  (call   main (: 4 Int64)) (output (: 174 Int64)))
