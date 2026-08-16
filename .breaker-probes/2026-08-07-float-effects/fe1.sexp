(case "fe1 a Float64 handler state HALVING per dispatch — three draws sum to exactly 1.75x the seed (exact binary fractions)"
  (input  (do
            (effect F (op next (-> Float64)))
            (def (main (: n Int64))
              (handle F (Float64.of-int n)
                ((next () s (resume s (* s 0.5))))
                (let ((a (F.next)))
                  (let ((b (F.next)))
                    (let ((c (F.next)))
                      (if (= (+ a (+ b c)) (* (Float64.of-int n) 1.75)) 1 0))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 1 Int64))
  (call   main (: -12 Int64)) (output (: 1 Int64))
  (call   main (: 0 Int64)) (output (: 1 Int64)))
