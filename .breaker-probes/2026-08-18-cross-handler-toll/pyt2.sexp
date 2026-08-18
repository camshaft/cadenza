(case "pyt2 the FOREIGN LEVY INSIDE THE RESUME'S ANSWER ARGUMENT — each inner tick levies the outer handler WHILE BUILDING its answer so the levies fire in DISPATCH order, the exact mirror of the post-resume toll whose levies fire in unwind order, and the pair pins that argument-position performs precede the suspend while post-resume performs follow the replay"
  (input  (do
            (effect T (op levy (-> Int64)))
            (effect E (op tick (-> Int64)))
            (def (main (: n Int64))
              (handle T (% n 3)
                ((levy () t (resume t (+ t 5))))
                (handle E (: 1 Int64)
                  ((tick () s
                    (resume (+ s (T.levy)) (+ s 1))))
                  (let ((a (E.tick)))
                    (let ((b (E.tick)))
                      (+ a (* 10 b)))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 82 Int64))
  (call   main (: 0 Int64)) (output (: 71 Int64)))
