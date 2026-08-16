(case "aa1 TWO arms of one handler share a helper fn (arm-level code reuse through the lowering)"
  (input  (do
            (effect St (op a (-> Int64 Int64)) (op b (-> Int64 Int64)))
            (def (score (: v Int64) (: s Int64)) (+ (* 10 v) s))
            (def (main (: n Int64))
              (handle St n
                ((a (v) s (resume (score v s) (+ s 1)))
                 (b (v) s (resume (* 2 (score v s)) s)))
                (+ (St.a 3) (St.b 4))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 127 Int64)))
