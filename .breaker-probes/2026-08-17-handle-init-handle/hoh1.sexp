(case "hoh1 a HANDLE WHOSE INIT IS ITSELF A WHOLE HANDLE EXPRESSION — the outer Fibonacci pair-walker's starting tuple is computed by an inner two-dispatch counter handle that answers seed-plus-one while doubling its own state, both inner draws land in the outer init tuple, the inner handler is fully torn down before the outer installs, and the seed steers the inner counter so the two runs walk Fibonacci from different gates"
  (input  (do
            (effect B (op step (-> Int64)))
            (effect F (op draw (-> Int64)))
            (def (main (: n Int64))
              (handle F (handle B (% n 3)
                          ((step () s (resume (+ s 1) (* s 2))))
                          (tuple (B.step) (B.step)))
                ((draw () st
                  (match st
                    ((tuple x y)
                      (resume (+ (* x 10) y) (tuple y (+ x y)))))))
                (let ((a (F.draw)))
                  (let ((b (F.draw)))
                    (let ((c (F.draw)))
                      (let ((d (F.draw)))
                        (+ (* 100 (+ (* 100 (+ (* 100 a) b)) c)) d)))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 23355893 Int64))
  (call   main (: 0 Int64)) (output (: 11122335 Int64)))
