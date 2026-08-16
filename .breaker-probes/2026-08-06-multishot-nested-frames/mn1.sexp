(case "mn1 a multi-shot continuation contains a NESTED handler — each re-reduction re-enters it fresh"
  (input  (do
            (effect Amb (op flip (-> Unit Int64)))
            (effect In (op get (-> Unit Int64)))
            (def (main (: n Int64))
              (handle Amb 0
                ((flip (u) s (+ (resume 10 s) (resume 20 s))))
                (+ (Amb.flip)
                   (handle In 7
                     ((get (u) t (resume t (+ t 1))))
                     (In.get)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 44 Int64)))
