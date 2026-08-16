(case "pk1 a BEGIN/END protocol arm — a Bool flag rejects double-begin and end-without-begin, the sequence encodes the violations"
  (input  (do
            (effect E (op begin (-> Int64)) (op end (-> Int64)))
            (def (main (: n Int64))
              (handle E false
                ((begin () open (if open (resume -1 open) (resume 1 true)))
                 (end () open (if open (resume 7 false) (resume -1 open))))
                (let ((r1 (E.begin)))
                  (let ((r2 (E.begin)))
                    (let ((r3 (E.end)))
                      (let ((r4 (E.end)))
                        (+ (* 1000 r1) (+ (* 100 r2) (+ (* 10 r3) r4)))))))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 969 Int64)))
