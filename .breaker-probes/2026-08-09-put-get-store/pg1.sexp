(case "pg1 a PUT/GET store — put returns the value it displaces, a counter tracks writes, get reads the survivor"
  (input  (do
            (effect E (op put (-> Int64 Int64)) (op get (-> Int64)) (op writes (-> Int64)))
            (def (main (: n Int64))
              (handle E (tuple 0 0)
                ((put (x) s (match s
                              ((tuple last ctr) (resume last (tuple x (+ ctr 1))))))
                 (get () s (match s ((tuple last ctr) (resume last s))))
                 (writes () s (match s ((tuple last ctr) (resume ctr s)))))
                (let ((r1 (E.put (* 10 n))))
                  (let ((r2 (E.put 7)))
                    (+ r1 (+ r2 (+ (* 100 (E.get)) (* 1000 (E.writes)))))))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 2730 Int64))
  (call   main (: 0 Int64)) (output (: 2700 Int64))
  (call   main (: -2 Int64)) (output (: 2680 Int64)))
