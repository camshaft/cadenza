(case "ax12-ctl the resuming control: both bindings resume — earlier advance survives (proves 110 right)"
  (input  (do
            (effect A (op tick (-> Unit Int64)) (op get (-> Unit Int64)))
            (effect B (op bump (-> Unit Int64)))
            (def (main (: n Int64))
              (handle A 10
                ((tick (u) s (resume s (+ s 1))) (get (u) s (resume s s)))
                (let ((b (handle B 0 ((bump (u) s (resume s (+ s 1)))) (let ((y (A.tick)) (x (+ 1 (B.bump)))) (+ x y)))))
                  (+ b (A.get)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 22 Int64)))
