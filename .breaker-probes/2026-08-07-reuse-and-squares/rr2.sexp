(case "rr2 the SQUARE of a difference of two draws — composite arithmetic over one advancing thread, zero row included"
  (input  (do
            (effect St (op next (-> Int64)))
            (def (main (: n Int64))
              (handle St n
                ((next () s (resume s (* s 3))))
                (let ((a (St.next)))
                  (let ((b (St.next)))
                    (* (- b a) (- b a))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 100 Int64))
  (call   main (: -2 Int64)) (output (: 16 Int64))
  (call   main (: 0 Int64)) (output (: 0 Int64)))
