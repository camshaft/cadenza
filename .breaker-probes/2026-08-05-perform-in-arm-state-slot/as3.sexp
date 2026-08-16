(case "as3 minimal: does the state-slot perform RUN at all? (arm value from A.get directly)"
  (input  (do
            (effect A (op get (-> Unit Int64)))
            (effect B (op step (-> Unit Int64)))
            (def (main (: n Int64))
              (handle A n
                ((get (u) s (resume s (+ s 1))))
                (handle B 0
                  ((step (u) t (resume (A.get) t)))
                  (+ (* 10 (B.step)) (A.get)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 56 Int64)))
