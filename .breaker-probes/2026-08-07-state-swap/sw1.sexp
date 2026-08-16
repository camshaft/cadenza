(case "sw1 the state-SWAP idiom — the op argument becomes the state and the OLD state returns"
  (input  (do
            (effect St (op swap (-> (List Int64) (List Int64))))
            (def (main (: n Int64))
              (handle St (list 1 2)
                ((swap (xs) s (resume s xs)))
                (let ((old (St.swap (list n 7 8 9))))
                  (let ((cur (St.swap (list))))
                    (+ (* 100 (List.len old)) (List.len cur))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 204 Int64)))
