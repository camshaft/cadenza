(case "ib1 a draw ROUTES among three sibling ops — each arm transforms the live state differently, a probe pins the shared advance"
  (input  (do
            (effect E (op pick (-> Int64)) (op tens (-> Int64)) (op plus (-> Int64)) (op negs (-> Int64)) (op probe (-> Int64)))
            (def (main (: n Int64))
              (handle E n
                ((pick () s (resume (% s 3) (+ s 1)))
                 (tens () s (resume (* s 10) (+ s 1)))
                 (plus () s (resume (+ s 500) (+ s 1)))
                 (negs () s (resume (- 0 s) (+ s 1)))
                 (probe () s (resume s s)))
                (let ((i (E.pick)))
                  (+ (* 10 (if (= i 0) (E.tens) (if (= i 1) (E.plus) (E.negs))))
                     (- (E.probe) n)))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 402 Int64))
  (call   main (: 4 Int64)) (output (: 5052 Int64))
  (call   main (: 2 Int64)) (output (: -28 Int64)))
