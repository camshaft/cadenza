(case "pyk1 an INDEX-WEIGHTED post-resume toll — the state pairs a value with a dispatch counter and each frame's toll is their PRODUCT so the very first frame's toll is zeroed by its own index while the second frame pays value-times-one, a product of two captured fields distinguishing which frame's pair fed which toll beyond what either field alone could"
  (input  (do
            (effect E (op tick (-> Int64)))
            (def (main (: n Int64))
              (handle E (tuple (% n 3) (: 0 Int64))
                ((tick () st
                  (match st
                    ((tuple v k)
                      (+ (resume v (tuple (+ v 3) (+ k 1)))
                         (* 100 (* v k)))))))
                (let ((a (E.tick)))
                  (let ((b (E.tick)))
                    (+ a (* 10 b))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 441 Int64))
  (call   main (: 0 Int64)) (output (: 330 Int64)))
