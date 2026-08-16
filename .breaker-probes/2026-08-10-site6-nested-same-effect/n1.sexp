(case "n1 block-wrapped branch-perform inside an INNER re-handle of the SAME effect — the float must attribute the perform to the inner handler"
  (input  (do
            (effect St (op get (-> Int64)))
            (def (main (: n Int64))
              (handle St n
                ((get () s (resume s (+ s 1))))
                (let ((outer1 (St.get))
                      (inner (handle St 500
                               ((get () s (resume s (+ s 10))))
                               (let ((v (let ((b 1)) (if (= b 1) (St.get) 77))))
                                 (+ v (St.get))))))
                  (+ (* 1000 inner) (+ (* 10 outer1) (St.get))))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 1010034 Int64)))
