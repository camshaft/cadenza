(case "pyv1 the POST-RESUME TOLL KEYED TO THE OP ARGUMENT — each dispatch answers argument-plus-state threading their sum then adds a hundredfold toll of ITS OWN argument after the replay, the two arguments differ so the unwinding tolls must each recall the argument their dispatch received across the suspend, and a toll reading the other frame's argument or the state instead shifts the hundreds"
  (input  (do
            (effect E (op tick (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle E (% n 3)
                ((tick (v) s
                  (+ (resume (+ v s) (+ s v)) (* 100 v))))
                (let ((a (E.tick 4)))
                  (let ((b (E.tick 7)))
                    (+ a (* 10 b))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 1225 Int64))
  (call   main (: 0 Int64)) (output (: 1214 Int64)))
