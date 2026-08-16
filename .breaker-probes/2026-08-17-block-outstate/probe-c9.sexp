(case "c9 let-bound bool if pure branches control"
  (input  (do
            (def (main (: x Int64))
              (let ((v (let ((b true)) (if b 5 9))))
                (+ v x)))
            (export main)))
  (call   main (: 3 Int64)) (output (: 8 Int64)))
