(case "s6d the floated wrapper init can trap — division by the argument stays ORDERED before the conditional"
  (input  (do
            (effect St (op get (-> Int64)))
            (def (main (: n Int64))
              (handle St n
                ((get () s (resume s (+ s 1))))
                (let ((v (let ((t (/ 100 n))) (if (= t 25) (St.get) t))))
                  (+ (* 10 v) (St.get)))))
            (export main)))
  (call   main (: 4 Int64)) (output (: 45 Int64))
  (call   main (: 5 Int64)) (output (: 205 Int64))
  (call   main (: 0 Int64)) (trap "divide by zero"))
