(case "dv3 division by -1 of a RUNTIME draw — negation across the full non-MIN range, the MIN draw guarded to its own branch"
  (input  (do
            (effect E (op next (-> Int64)))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s s)))
                (let ((x (E.next)))
                  (if (= x -9223372036854775808)
                      777
                      (/ x -1)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: -5 Int64))
  (call   main (: -9223372036854775807 Int64)) (output (: 9223372036854775807 Int64))
  (call   main (: -9223372036854775808 Int64)) (output (: 777 Int64)))
