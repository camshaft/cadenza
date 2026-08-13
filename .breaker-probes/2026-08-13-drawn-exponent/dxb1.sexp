(case "dxb1 modular exponentiation with the EXPONENT BITS drawn from the handler — the body's square-and-multiply loop consumes the bit stream LSB-first while the arm peels the threaded exponent"
  (input  (do
            (effect S (op bit (-> Int64)))
            (def (powstep (: k Int64) (: result Int64) (: power Int64))
              (if (< k 1)
                  result
                  (let ((b (S.bit)))
                    (powstep (- k 1)
                             (if (= b 1) (% (* result power) 101) result)
                             (% (* power power) 101)))))
            (def (main (: n Int64))
              (handle S n
                ((bit () s (resume (% s 2) (/ s 2))))
                (powstep 4 1 3)))
            (export main)))
  (call   main (: 5 Int64)) (output (: 41 Int64))
  (call   main (: 12 Int64)) (output (: 80 Int64)))
