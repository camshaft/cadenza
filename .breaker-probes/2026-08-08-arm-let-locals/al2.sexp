(case "al2 a PURE helper called from the arm for BOTH slots — the arm delegates its computation to a def"
  (input  (do
            (effect E (op f (-> Int64 Int64)))
            (def (mix (: a Int64) (: b Int64)) (+ (* a a) b))
            (def (main (: n Int64))
              (handle E n
                ((f (v) s (resume (mix v s) (mix s 1))))
                (+ (E.f 2) (E.f 3))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 44 Int64))
  (call   main (: 0 Int64)) (output (: 14 Int64)))
