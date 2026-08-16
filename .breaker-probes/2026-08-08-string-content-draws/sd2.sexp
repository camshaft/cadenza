(case "sd2 draw parity picks WHICH string each op returns — the concat order of two draws is visible in content equality"
  (input  (do
            (effect E (op pick (-> String)))
            (def (main (: n Int64))
              (handle E n
                ((pick () s (resume (if (= (% s 2) 0) "xy" "pqr") (+ s 1))))
                (let ((a (E.pick)))
                  (let ((b (E.pick)))
                    (let ((st (String.concat a b)))
                      (+ (* 100 (String.byte-len st))
                         (if (= st "xypqr") 10 (if (= st "pqrxy") 20 30))))))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 510 Int64))
  (call   main (: 1 Int64)) (output (: 520 Int64)))
