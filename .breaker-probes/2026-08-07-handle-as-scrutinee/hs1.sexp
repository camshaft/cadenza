(case "hs1 an inner SAME-effect handle's result is the match SCRUTINEE — the selected arm and the trailing draw hit the OUTER state"
  (input  (do
            (effect St (op next (-> Int64)))
            (def (main (: n Int64))
              (handle St n
                ((next () s (resume s (+ s 1))))
                (+ (match (handle St 4
                            ((next () t (resume t (* t 3))))
                            (+ (St.next) (St.next)))
                     (16 (+ 1000 (St.next)))
                     (_o (- 0 _o)))
                   (St.next))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 1011 Int64))
  (call   main (: 0 Int64)) (output (: 1001 Int64)))
