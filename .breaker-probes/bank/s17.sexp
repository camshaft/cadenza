(case "s17 RECORD field holding the closure + direct call"
  (input  (do
            (def (main (: d Int64))
              (let ((k 100))
                (let ((f1 (fn ((: v Int64)) (+ k v))))
                  (let ((r (record (f f1) (t 1))))
                    (+ (f1 d) (. r t))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 106 Int64)))
