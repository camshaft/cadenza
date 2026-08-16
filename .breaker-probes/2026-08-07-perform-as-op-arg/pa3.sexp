(case "pa3 THREE-deep nested same-effect arg-feed — scale(scale(next)) with a state-advancing scale arm"
  (input  (do
            (effect St (op next (-> Int64)) (op scale (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle St n
                ((next () s (resume s (+ s 1)))
                 (scale (v) s (resume (+ (* 10 v) s) (+ s 1))))
                (St.scale (St.scale (St.next)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 567 Int64))
  (call   main (: 2 Int64)) (output (: 234 Int64)))
