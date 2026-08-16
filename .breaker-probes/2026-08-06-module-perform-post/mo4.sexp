(case "mo4 a module-exported recursive performer called from a handler ARM (arm→module→recursion)"
  (input  (do
            (effect Ctr (op next (-> Unit Int64)))
            (effect Ask (op get (-> Unit Int64)))
            (module m
              (def (walk (: n Int64) (: acc Int64))
                (if (= n 0) acc (walk (- n 1) (+ acc (Ctr.next unit)))))
              (export walk))
            (def (main (: k Int64))
              (handle Ctr 10 ((next (u) s (resume s (+ s 1))))
                (handle Ask 0 ((get (u) s (resume ((. m walk) k 0) s)))
                  (Ask.get))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 33 Int64)))
