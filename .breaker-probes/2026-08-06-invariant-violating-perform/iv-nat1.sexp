(case "iv-nat1 the NATURAL invariant path: (Percent.Pct (St.next)) under a handle — in-range runs"
  (input  (do
            (effect St (op next (-> Unit Int64)))
            (@ (invariant (and (>= self 0) (<= self 100))) (type Percent (Pct Int64)))
            (def (unwrap (: p Percent)) (match p (((. Percent Pct) n) n)))
            (def (main (: n Int64))
              (handle St 42
                ((next (u) s (resume s (+ s 1))))
                (unwrap (Percent.Pct (St.next)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 42 Int64)))
