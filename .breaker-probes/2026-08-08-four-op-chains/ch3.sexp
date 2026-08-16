(case "ch3 the chain hops CROSS two effects — F.b of G.p of F.a, each thread advancing only on its own hops"
  (input  (do
            (effect F (op a (-> Int64)) (op b (-> Int64 Int64)) (op fp (-> Int64)))
            (effect G (op p (-> Int64 Int64)) (op gp (-> Int64)))
            (def (main (: n Int64))
              (handle F n
                ((a () s (resume s (+ s 2)))
                 (b (x) s (resume (+ x s) (+ s 3)))
                 (fp () s (resume s s)))
                (handle G 100
                  ((p (x) t (resume (+ x t) (+ t 10)))
                   (gp () t (resume t t)))
                  (+ (* 10 (F.b (G.p (F.a)))) (+ (F.fp) (G.gp))))))
            (export main)))
  (call   main (: 2 Int64)) (output (: 1177 Int64))
  (call   main (: 0 Int64)) (output (: 1135 Int64))
  (call   main (: -6 Int64)) (output (: 1009 Int64)))
