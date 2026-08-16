(case "ab1d the scalar twin — a conditional abort under a SCALAR-state outer, pre-abort advance committed"
  (input  (do
            (effect L (op emit (-> Int64)))
            (effect Bail (op bail (-> Int64 Int64)))
            (def (main (: n Int64))
              (+ (handle L 10
                   ((emit () s (resume s (+ s 1))))
                   (handle Bail 0
                     ((bail (v) s v))
                     (do
                       (L.emit)
                       (let ((g (if (> n 3) (Bail.bail 99) 0)))
                         (+ g (+ (L.emit) 500))))))
                 (* 1000 n)))
            (export main)))
  (call   main (: 5 Int64)) (output (: 5099 Int64))
  (call   main (: 0 Int64)) (output (: 511 Int64)))
