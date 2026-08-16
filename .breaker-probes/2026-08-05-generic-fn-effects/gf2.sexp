(case "gf2 a generic helper applied at TWO types under one handler (Int64 perform result + String)"
  (input  (do
            (effect Cnt (op bump (-> Unit Int64)))
            (def (idem x) x)
            (def (main (: n Int64))
              (handle Cnt n
                ((bump (u) s (resume s (+ s 1))))
                (+ (idem (Cnt.bump))
                   (String.scalar-len (idem "abc")))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 8 Int64)))
