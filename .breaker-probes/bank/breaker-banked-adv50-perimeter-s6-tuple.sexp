(case "s6 capturing closure in a TUPLE + direct call"
  (input  (do
            (def (main (: d Int64))
              (let ((xs (list 7 8 9)))
                (let ((f1 (fn ((: v Int64)) (+ (* (List.len xs) 100) v))))
                  (let ((t (tuple f1 1)))
                    (+ (f1 d) ((. t 0) d))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 610 Int64)))
