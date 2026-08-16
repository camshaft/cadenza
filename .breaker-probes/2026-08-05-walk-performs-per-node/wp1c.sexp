(case "wp1c control: NON-branching recursion performing per step (the rw-class shape, known green)"
  (input  (do
            (effect Cnt (op bump (-> Unit Int64)))
            (def (loop (: n Int64))
              (if (= n 0) 0 (do (def _b (Cnt.bump)) (+ 1 (loop (- n 1))))))
            (def (main (: n Int64))
              (handle Cnt 0
                ((bump (u) s (resume s (+ s 1))))
                (loop n)))
            (export main)))
  (call   main (: 5 Int64)) (output (: 5 Int64)))
