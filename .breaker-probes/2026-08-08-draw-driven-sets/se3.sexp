(case "se3 a set BUILT from cycling draws probed by a LATER state read — the in-cycle probe hits, the off-cycle probe misses"
  (input  (do
            (effect E (op next (-> Int64)) (op probe (-> Int64)))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (% (+ s 2) 4)))
                 (probe () s (resume s s)))
                (let ((st (Set.of (list (E.next) (E.next) (E.next)))))
                  (let ((p (E.probe)))
                    (+ (* 1000 (if (Set.contains st p) 1 5))
                       (+ (* 100 (if (Set.contains st (+ p 1)) 1 5))
                          (+ (* 10 (Set.len st)) p)))))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 1522 Int64))
  (call   main (: 1 Int64)) (output (: 1523 Int64))
  (call   main (: 6 Int64)) (output (: 1530 Int64)))
