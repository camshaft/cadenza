(case "s16 plain list LITERAL holding the closure + direct call"
  (input  (do
            (def (main (: d Int64))
              (let ((k 100))
                (let ((f1 (fn ((: v Int64)) (+ k v))))
                  (let ((fs (list f1)))
                    (+ (f1 d) (List.len fs))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 106 Int64)))
