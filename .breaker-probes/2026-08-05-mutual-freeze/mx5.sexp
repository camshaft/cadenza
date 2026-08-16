(case "mx5 the filed freeze shape: partner-seeded empty list + element from partner's tuple"
  (input  (do
            (type Tok (A Int64) (B Int64))
            (def (dn (: n Int64))
              (if (= n 0) (tuple (A 0) 0) (dac n (- n 1) (list))))
            (def (dac (: n Int64) (: i Int64) acc)
              (if (= i 0) (tuple (A n) (List.len acc))
                  (match (dn (- i 1))
                    ((tuple child _nx) (dac n (- i 1) (List.push acc child))))))
            (def (main (: k Int64))
              (match (dn k) ((tuple _t len) len)))
            (export main)))
  (call   main (: 3 Int64)) (output (: 2 Int64)))
