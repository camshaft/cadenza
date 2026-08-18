(case "pysh5 REPEATED SELF-PERFORMS THREAD BOTH STATE LADDERS — two shadowed draws each route their arm's inner self-perform to the outer handler so the outer state ladder advances once per inner dispatch while the inner state doubles independently, both ladders' progressions land in both answers, and either ladder stalling or the self-perform re-entering the inner region misprices a distinct digit range"
  (input  (do
            (effect E (op tick (-> Int64)))
            (def (main (: n Int64))
              (handle E (% n 3)
                ((tick () s (resume (+ (* s 10) 1) (+ s 1))))
                (handle E (: 50 Int64)
                  ((tick () s (resume (+ s (E.tick)) (* s 2))))
                  (+ (E.tick) (* 1000 (E.tick))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 121061 Int64))
  (call   main (: 0 Int64)) (output (: 111051 Int64)))
