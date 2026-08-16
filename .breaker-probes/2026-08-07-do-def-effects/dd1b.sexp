(case "dd1b consecutive do-DEF draws — both binders hold their reads, the tail draw sees the doubled-twice state"
  (input  (do
            (effect St (op next (-> Int64)))
            (def (main (: n Int64))
              (handle St n
                ((next () s (resume s (* s 2))))
                (do
                  (def a (St.next))
                  (def b (St.next))
                  (+ (* 100 a) (+ (* 10 b) (St.next))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 620 Int64))
  (call   main (: 1 Int64)) (output (: 124 Int64)))
