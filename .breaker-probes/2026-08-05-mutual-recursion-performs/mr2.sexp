(case "mr2 mutual recursion where only ONE side performs (the other is a pure relay)"
  (input  (do
            (effect Cnt (op tick (-> Unit Int64)))
            (def (walk (: k Int64))
              (if (= k 0) 0 (+ (Cnt.tick) (relay (- k 1)))))
            (def (relay (: k Int64))
              (if (= k 0) 100 (walk k)))
            (def (main (: n Int64))
              (handle Cnt n
                ((tick (u) s (resume s (+ s 1))))
                (walk 3)))
            (export main)))
  (call   main (: 5 Int64)) (output (: 118 Int64)))
