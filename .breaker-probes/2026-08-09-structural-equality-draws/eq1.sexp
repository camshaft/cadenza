(case "eq1 structural EQUALITY of records built from two draws — a parity-dependent stride decides whether the fields line up"
  (input  (do
            (effect E (op next (-> Int64)) (op probe (-> Int64)))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (if (= (% s 2) 0) (+ s 2) (+ s 1))))
                 (probe () s (resume s s)))
                (let ((a (E.next)))
                  (let ((b (E.next)))
                    (+ (if (= (record (x a) (y (+ a 1))) (record (x a) (y b))) 100 200)
                       (- (E.probe) n))))))
            (export main)))
  (call   main (: 4 Int64)) (output (: 204 Int64))
  (call   main (: 3 Int64)) (output (: 103 Int64))
  (call   main (: 7 Int64)) (output (: 103 Int64)))
