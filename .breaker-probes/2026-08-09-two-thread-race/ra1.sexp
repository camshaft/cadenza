(case "ra1 a RACE between two effect threads — a recursive walk draws BOTH per round until the fast thread catches the slow one's head start"
  (input  (do
            (effect A (op next (-> Int64)))
            (effect B (op next (-> Int64)))
            (def (race (: steps Int64))
              (let ((a (A.next)))
                (let ((b (B.next)))
                  (if (< a b) (race (+ steps 1)) (+ (* 100 steps) (- a b))))))
            (def (main (: n Int64))
              (handle A n
                ((next () s (resume (+ s 5) (+ s 5))))
                (handle B (+ n (+ (* 2 (if (< (% n 5) 0) (- 0 (% n 5)) (% n 5))) 3))
                  ((next () t (resume (+ t 2) (+ t 2))))
                  (race 0))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 0 Int64))
  (call   main (: 1 Int64)) (output (: 101 Int64))
  (call   main (: -4 Int64)) (output (: 301 Int64)))
