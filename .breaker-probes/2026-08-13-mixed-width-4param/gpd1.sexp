(case "gpd1 the match GUARD itself PERFORMS — the guard's draw advances the thread before the branch commits, and the trailing draw exposes the total advance"
  (input  (do
            (effect S (op next (-> Int64)))
            (def (main (: n Int64))
              (handle S n
                ((next () s (resume s (+ s 2))))
                (let ((r (match (S.next)
                           ((guard v (< v (S.next))) 1)
                           (_v 0))))
                  (+ (* 1000 r) (S.next)))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 1007 Int64))
  (call   main (: -5 Int64)) (output (: 999 Int64)))
