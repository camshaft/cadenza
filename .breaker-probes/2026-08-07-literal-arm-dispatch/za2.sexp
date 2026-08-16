(case "za2 a COMPUTED scrutinee (difference of two draws) selects the performing arm at one input only"
  (input  (do
            (effect St (op next (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St n
                ((next (u) s (resume s (* s 2))))
                (let ((a (St.next)))
                  (let ((b (St.next)))
                    (match (- b a)
                      (5 (+ 1000 (St.next)))
                      (_o (- 0 (- b a))))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 1020 Int64))
  (call   main (: 3 Int64)) (output (: -3 Int64)))
