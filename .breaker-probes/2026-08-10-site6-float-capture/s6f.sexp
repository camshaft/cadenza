(case "s6f block-wrapped branch-perform as a MATCH scrutinee through the float"
  (input  (do
            (effect St (op get (-> Int64)))
            (def (main (: n Int64))
              (handle St n
                ((get () s (resume s (+ s 1))))
                (let ((v (let ((k (% n 2))) (if (= k 0) (St.get) 50))))
                  (match v
                    (4 (+ 300 (St.get)))
                    (50 (+ 500 (St.get)))
                    (_ (St.get))))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 503 Int64))
  (call   main (: 4 Int64)) (output (: 305 Int64))
  (call   main (: 8 Int64)) (output (: 9 Int64)))
