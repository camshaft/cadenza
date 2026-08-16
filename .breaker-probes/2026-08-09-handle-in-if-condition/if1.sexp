(case "if1 a WHOLE nested handle expression as an IF's CONDITION beside outer draws"
  (input  (do
            (effect E (op next (-> Int64)))
            (effect B (op g (-> Unit Int64)))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 1))))
                (if (> (handle B 0 ((g (u) t (resume t t))) (+ (B.g) (E.next))) 0)
                    (+ 100 (E.next))
                    (- (E.next) 100))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 106 Int64))
  (call   main (: 0 Int64)) (output (: -99 Int64))
  (call   main (: -7 Int64)) (output (: -106 Int64)))
