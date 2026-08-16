(case "cc3b control: PURE closure through the same helper under the same handle"
  (input  (do
            (effect Cnt (op bump (-> Unit Int64)))
            (def (apply-twice (: g (-> Int64 Int64)))
              (+ (g 10) (g 100)))
            (def (main (: n Int64))
              (handle Cnt n
                ((bump (u) s (resume s (+ s 1))))
                (+ (apply-twice (fn ((: k Int64)) (* k 2))) (Cnt.bump))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 225 Int64)))
