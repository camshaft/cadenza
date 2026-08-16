(case "rd2 performs inside a TUPLE literal then projected (position-value checksum)"
  (input  (do
            (effect St (op next (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St n
                ((next (u) s (resume s (+ s 1))))
                (do
                  (def t (tuple (St.next) (St.next) (St.next)))
                  (+ (* 100 (. t 0)) (+ (* 10 (. t 1)) (. t 2))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 567 Int64)))
