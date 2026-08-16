(case "px2 TWO observed walks back-to-back — the second walk re-enters the upgraded loop after the first drain"
  (input  (do
            (effect Acc (op push (-> Int64 Int64)) (op size (-> Int64)))
            (def (grow (: n Int64))
              (if (< n 1) 0 (match (Acc.push n) (_ (grow (- n 1))))))
            (def (main (: n Int64))
              (handle Acc 0
                ((push (v) s (resume s (+ s 1)))
                 (size () s (resume s s)))
                (let ((g1 (grow n)))
                  (let ((d1 (Acc.size)))
                    (let ((g2 (grow n)))
                      (+ (* 100 d1) (Acc.size)))))))
            (export main)))
  (call   main (: 50 Int64)) (output (: 5100 Int64))
  (call   main (: 0 Int64)) (output (: 0 Int64)))
