(case "pyv2 a NON-COMMUTATIVE DIFFERENCE TOLL — each frame charges a hundredfold of the op argument MINUS the captured state so swapping the operands negates the toll, the two dispatches subtract different pairs, and the seed that RAISES the state SHRINKS the answer through both differences at once"
  (input  (do
            (effect E (op tick (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle E (% n 3)
                ((tick (v) s
                  (+ (resume (+ v s) (+ s 1)) (* 100 (- v s)))))
                (let ((a (E.tick 7)))
                  (let ((b (E.tick 4)))
                    (+ a (* 10 b))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 868 Int64))
  (call   main (: 0 Int64)) (output (: 1057 Int64)))
