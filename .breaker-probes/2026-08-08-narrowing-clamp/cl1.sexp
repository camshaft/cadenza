(case "cl1 a NARROWING clamp arm — the [lo,hi] window shrinks by one each side per dispatch, three args meet three windows"
  (input  (do
            (effect E (op clamp (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle E (tuple 0 10)
                ((clamp (x) s (match s
                                ((tuple l h)
                                  (resume (if (< x l) l (if (> x h) h x))
                                          (tuple (+ l 1) (- h 1)))))))
                (let ((a (E.clamp n)))
                  (let ((b (E.clamp 5)))
                    (let ((c (E.clamp (+ n 3))))
                      (+ (* 100 a) (+ (* 10 b) c)))))))
            (export main)))
  (call   main (: 7 Int64)) (output (: 758 Int64))
  (call   main (: -4 Int64)) (output (: 52 Int64))
  (call   main (: 12 Int64)) (output (: 1058 Int64)))
