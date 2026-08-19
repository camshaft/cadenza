(case "pyv3 a MODULO TOLL OVER BOTH CAPTURES — each frame charges a hundredfold of the op argument REDUCED modulo the state-plus-one so the toll is a nonlinear mix of both captures, the two frames reduce different pairs to small residues, and either a swapped operand order or a cross-frame pair shifts the hundreds by the wrong residue"
  (input  (do
            (effect E (op tick (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle E (% n 3)
                ((tick (v) s
                  (+ (resume (+ v s) (+ s 1)) (* 100 (% v (+ s 1))))))
                (let ((a (E.tick 7)))
                  (let ((b (E.tick 5)))
                    (+ a (* 10 b))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 378 Int64))
  (call   main (: 0 Int64)) (output (: 167 Int64)))
