(case "cn1 a RESUMPTIVE perform in a connective's RHS runs only on the reached path, state observed"
  (input  (do
            (effect Ctr (op tick (-> Unit Int64)))
            (def (main (: b Bool))
              (handle Ctr 5 ((tick (u) s (resume s (+ s 1))))
                (+ (if (and b (> (Ctr.tick) 4)) 100 200)
                   (Ctr.tick))))
            (export main)))
  (call   main (: true Bool)) (output (: 106 Int64))
  (call   main (: false Bool)) (output (: 205 Int64)))
