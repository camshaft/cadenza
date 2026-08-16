(case "mg1 a GENERIC helper wraps the multi-site perform (effect-specialized generic through the refold)"
  (input  (do
            (effect St (op sift (-> Int64 Int64)))
            (def (both f a b) (+ (f a) (f b)))
            (def (main (: n Int64))
              (handle St 0
                ((sift (v) s (if (> v 10) (resume v (+ s 1)) (resume 0 s))))
                (both (fn ((: x Int64)) (St.sift x)) 20 n)))
            (export main)))
  (call   main (: 30 Int64)) (output (: 50 Int64)))
