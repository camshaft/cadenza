(case "cn1 a THREE-deep seed RELAY — each nested shadow's seed is a draw from its parent, strides differ per depth"
  (input  (do
            (effect St (op next (-> Int64)))
            (def (main (: n Int64))
              (handle St n
                ((next () s (resume s (+ s 1))))
                (handle St (St.next)
                  ((next () s (resume s (+ s 10))))
                  (handle St (St.next)
                    ((next () s (resume s (+ s 100))))
                    (+ (St.next) (St.next))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 110 Int64))
  (call   main (: 0 Int64)) (output (: 100 Int64)))
