(case "pn1 a Peano user-sum built to a perform-determined depth, folded back under the same handler"
  (input  (do
            (type Nat (Z) (S Nat))
            (effect St (op depth (-> Unit Int64)))
            (def (mk (: n Int64)) (if (= n 0) (Z) (S (mk (- n 1)))))
            (def (count (: x Nat))
              (match x ((Z) 0) ((S p) (+ 1 (count p)))))
            (def (main (: n Int64))
              (handle St n
                ((depth (u) s (resume s (+ s 1))))
                (+ (* 10 (count (mk (St.depth)))) (count (mk (St.depth))))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 34 Int64)))
