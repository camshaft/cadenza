(case "pyr4 an IF CONDITION DIRECTLY ON THE RESUME CALL — each arm tests whether the resumed rest-of-body value clears the line and answers small-plus-state when it does or a thousand-plus-state when it does not, the branch values are chosen so the outer frame always takes the OPPOSITE branch from the inner one, and the seed flips which branch the inner frame starts on so the two runs produce a thousandfold-apart answers from the same machine"
  (input  (do
            (effect E (op tick (-> Int64)))
            (def (main (: n Int64))
              (handle E (% n 3)
                ((tick () s
                  (if (> (resume s (+ s 3)) 35)
                      (+ s 1)
                      (+ 1000 s))))
                (let ((a (E.tick)))
                  (let ((b (E.tick)))
                    (+ a (* 10 b))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 1001 Int64))
  (call   main (: 0 Int64)) (output (: 1 Int64)))
