(case "bc3 an op RETURNS Bool consumed by a two-flag if ladder — a mod-3-dependent step makes all four paths reachable"
  (input  (do
            (effect E (op flag (-> Bool)) (op probe (-> Int64)))
            (def (main (: n Int64))
              (handle E n
                ((flag () s (resume (= (% s 2) 0) (if (= (% s 3) 0) (+ s 1) (+ s 2))))
                 (probe () s (resume s s)))
                (let ((f1 (E.flag)))
                  (let ((f2 (E.flag)))
                    (+ (* 10 (if f1 (if f2 10 20) (if f2 30 40))) (E.probe))))))
            (export main)))
  (call   main (: 2 Int64)) (output (: 106 Int64))
  (call   main (: 0 Int64)) (output (: 203 Int64))
  (call   main (: 3 Int64)) (output (: 306 Int64))
  (call   main (: 1 Int64)) (output (: 404 Int64)))
