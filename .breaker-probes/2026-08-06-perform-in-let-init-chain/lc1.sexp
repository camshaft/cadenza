(case "lc1 a chain of perform-fed let inits — each feeds the next perform's argument"
  (input  (do
            (effect St (op add (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle St 0
                ((add (v) s (resume (+ v s) (+ s 1))))
                (let ((a (St.add n)))
                  (let ((b (St.add a)))
                    (let ((c (St.add b)))
                      (+ a (+ b c)))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 19 Int64)))
