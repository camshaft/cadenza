(case "dd2 a do-DEF bound to a whole nested SHADOW handle — the def holds the inner region's value, the tail draw reads outer"
  (input  (do
            (effect St (op next (-> Int64)))
            (def (main (: n Int64))
              (handle St n
                ((next () s (resume s (+ s 1))))
                (do
                  (def inner (handle St 40
                               ((next () t (resume t (* t 3))))
                               (+ (St.next) (St.next))))
                  (+ inner (St.next)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 165 Int64))
  (call   main (: 0 Int64)) (output (: 160 Int64)))
