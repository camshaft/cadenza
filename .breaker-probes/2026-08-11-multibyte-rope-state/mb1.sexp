(case "mb1 a MULTIBYTE rope state grows a 2-byte scalar per dispatch — byte-len and scalar-len diverge exactly at the 20-deep drain"
  (input  (do
            (effect S (op add (-> Int64)) (op dump (-> String)))
            (def (walk (: k Int64))
              (if (< k 1) 0 (let ((_d (S.add))) (walk (- k 1)))))
            (def (main (: n Int64))
              (handle S "x"
                ((add () s (resume 0 (String.concat s "é")))
                 (dump () s (resume s s)))
                (let ((_w (walk n)))
                  (let ((t (S.dump)))
                    (+ (* 1000 (String.byte-len t)) (String.scalar-len t))))))
            (export main)))
  (call   main (: 20 Int64)) (output (: 41021 Int64))
  (call   main (: 0 Int64)) (output (: 1001 Int64)))
