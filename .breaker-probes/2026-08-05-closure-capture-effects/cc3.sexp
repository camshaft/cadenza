(case "cc3 a performing closure passed INTO a helper that calls it (perform through an indirect call site)"
  (input  (do
            (effect Cnt (op bump (-> Unit Int64)))
            (def (apply-twice (: g (-> Int64 Int64)))
              (+ (g 10) (g 100)))
            (def (main (: n Int64))
              (handle Cnt n
                ((bump (u) s (resume s (+ s 1))))
                (apply-twice (fn ((: k Int64)) (* k (Cnt.bump))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 650 Int64)))
