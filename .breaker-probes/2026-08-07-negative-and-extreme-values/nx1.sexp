(case "nx1 a SUBTRACTING stride crosses zero — negative states thread and sum correctly"
  (input  (do
            (effect St (op next (-> Int64)))
            (def (main (: n Int64))
              (handle St n
                ((next () s (resume s (- s 7))))
                (+ (St.next) (+ (St.next) (+ (St.next) (St.next))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: -2 Int64))
  (call   main (: 0 Int64)) (output (: -42 Int64))
  (call   main (: -5 Int64)) (output (: -62 Int64)))
