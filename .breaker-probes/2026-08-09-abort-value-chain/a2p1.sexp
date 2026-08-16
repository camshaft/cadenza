(case "a2p1 the abort VALUE is a three-op chain — every hop advances the enclosing thread before the region tears down"
  (input  (do
            (effect E (op a (-> Int64)) (op b (-> Int64 Int64)) (op c (-> Int64 Int64)) (op probe (-> Int64)))
            (effect Bail (op out (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle E n
                ((a () s (resume s (+ s 2)))
                 (b (x) s (resume (+ x s) (+ s 3)))
                 (c (x) s (resume (* 2 x) (+ s 5)))
                 (probe () s (resume s s)))
                (+ (* 10 (handle Bail 0
                           ((out (v) t (+ 1000 v)))
                           (+ (Bail.out (E.c (E.b (E.a)))) 777)))
                   (- (E.probe) n))))
            (export main)))
  (call   main (: 2 Int64)) (output (: 10130 Int64))
  (call   main (: 0 Int64)) (output (: 10050 Int64))
  (call   main (: -4 Int64)) (output (: 9890 Int64)))
