(case "rq1 a RATIONAL handler state — three exact fractional adds (1/2 + 1/3 + 1/6) land on a WHOLE value — the canonical n/1 render pinned"
  (input  (do
            (effect E (op add (-> Int64 Int64)) (op report (-> Rational)))
            (def (main (: n Int64))
              (handle E (Rational.of n 1)
                ((add (d) s (resume 1 (+ s (Rational.of 1 d))))
                 (report () s (resume s s)))
                (do (E.add 2) (E.add 3) (E.add 6) (E.report))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 1/1 Rational))
  (call   main (: 1 Int64)) (output (: 2/1 Rational))
  (call   main (: -1 Int64)) (output (: 0/1 Rational)))
