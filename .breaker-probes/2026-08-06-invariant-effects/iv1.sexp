(case "iv1 an @invariant newtype constructed from PERFORM results (effects feed the checked type)"
  (input  (do
            (effect St (op next (-> Unit Int64)))
            (@ (invariant (and (>= self 0) (<= self 100))) (type Percent (Pct Int64)))
            (def (mk (: v Int64)) (Percent.Pct v))
            (def (unwrap (: p Percent)) (match p (((. Percent Pct) n) n)))
            (def (main (: n Int64))
              (handle St 42
                ((next (u) s (resume s (+ s 1))))
                (+ (unwrap (mk (St.next))) (unwrap (mk (St.next))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 85 Int64)))
