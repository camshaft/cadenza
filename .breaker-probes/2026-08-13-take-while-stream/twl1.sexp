(case "twl1 a TAKE-WHILE pull stream — the body's recursive driver keeps drawing until the arm hands back a divisible-by-four value, accumulating the survivors and counting all pulls including the terminator"
  (input  (do
            (effect S (op pull (-> Int64)))
            (def (drive (: k Int64) (: acc Int64) (: cnt Int64))
              (if (< k 1)
                  (+ (* 100 acc) cnt)
                  (let ((v (S.pull)))
                    (if (= (% v 4) 0)
                        (+ (* 100 acc) (+ cnt 1))
                        (drive (- k 1) (+ (* 100 acc) v) (+ cnt 1))))))
            (def (main (: n Int64))
              (handle S n
                ((pull () s (resume (% (* s s) 23) (+ s 1))))
                (drive 8 0 0)))
            (export main)))
  (call   main (: 3 Int64)) (output (: 902 Int64))
  (call   main (: 7 Int64)) (output (: 31803 Int64)))
