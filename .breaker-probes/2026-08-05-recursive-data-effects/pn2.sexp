(case "pn2 the Peano value AS the handler state, structurally grown per perform"
  (input  (do
            (type Nat (Z) (S Nat))
            (effect St (op up (-> Unit Int64)))
            (def (count (: x Nat))
              (match x ((Z) 0) ((S p) (+ 1 (count p)))))
            (def (main (: n Int64))
              (handle St (Z)
                ((up (u) s (resume (count s) (S s))))
                (+ (* 100 (St.up)) (+ (* 10 (St.up)) (St.up)))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 12 Int64)))
