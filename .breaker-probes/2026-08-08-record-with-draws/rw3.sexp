(case "rw3 each Record.with value PROJECTS from the record it updates plus a draw — self-referential functional updates chain through the thread"
  (input  (do
            (effect E (op next (-> Int64)))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 4))))
                (let ((r0 (record (x 1) (y 2))))
                  (let ((r1 (Record.with r0 #"x" (+ (* 10 (. r0 x)) (E.next)))))
                    (let ((r2 (Record.with r1 #"y" (+ (. r1 x) (+ (. r1 y) (E.next))))))
                      (+ (* 100 (. r2 x)) (. r2 y)))))))
            (export main)))
  (call   main (: 2 Int64)) (output (: 1220 Int64))
  (call   main (: 0 Int64)) (output (: 1016 Int64))
  (call   main (: -3 Int64)) (output (: 710 Int64)))
