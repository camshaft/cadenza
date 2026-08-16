(case "mf3 one performing helper called from an ARM's resume-value AND the body — three calls, one advancing thread"
  (input  (do
            (effect A (op a (-> Int64)))
            (effect B (op b (-> Int64)))
            (def (draw+) (+ (A.a) 1000))
            (def (main (: n Int64))
              (handle A n
                ((a () s (resume s (+ s 1))))
                (handle B 0
                  ((b () t (resume (draw+) t)))
                  (+ (B.b) (+ (draw+) (B.b))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 3018 Int64))
  (call   main (: 0 Int64)) (output (: 3003 Int64)))
