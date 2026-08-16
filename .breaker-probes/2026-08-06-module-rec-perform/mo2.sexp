(case "mo2 control: the same recursive performer OUTSIDE a module"
  (input  (do
            (effect Ctr (op next (-> Unit Int64)))
            (def (walk (: n Int64) (: acc Int64))
              (if (= n 0) acc (walk (- n 1) (+ (* 10 acc) (Ctr.next unit)))))
            (def (main (: k Int64))
              (handle Ctr 1
                ((next (u) s (resume s (+ s 1))))
                (walk k 0)))
            (export main)))
  (call   main (: 3 Int64)) (output (: 123 Int64)))
