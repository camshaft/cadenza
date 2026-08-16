(case "lo3 EIGHT ops in one effect, called in shuffled order — dispatch-table width, one op advancing mid-sequence"
  (input  (do
            (effect W (op a (-> Int64)) (op b (-> Int64)) (op c (-> Int64)) (op d (-> Int64))
                      (op e (-> Int64)) (op f (-> Int64)) (op g (-> Int64)) (op h (-> Int64)))
            (def (main (: n Int64))
              (handle W n
                ((a () s (resume (+ s 1) s))
                 (b () s (resume (+ s 2) s))
                 (c () s (resume (+ s 3) s))
                 (d () s (resume (+ s 4) s))
                 (e () s (resume (+ s 5) s))
                 (f () s (resume (+ s 6) s))
                 (g () s (resume (+ s 7) s))
                 (h () s (resume (+ s 8) (+ s 10))))
                (+ (W.a) (+ (W.h) (+ (W.b) (+ (W.g) (+ (W.c) (+ (W.f) (+ (W.d) (W.e))))))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 136 Int64))
  (call   main (: 0 Int64)) (output (: 96 Int64)))
