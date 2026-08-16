(case "ed1 TWO defs each install their OWN handler for one effect — main calls both, seeds and arms fully independent"
  (input  (do
            (effect St (op next (-> Int64)))
            (def (f1 (: n Int64))
              (handle St n
                ((next () s (resume s (+ s 1))))
                (+ (St.next) (St.next))))
            (def (f2 (: n Int64))
              (handle St (* n 10)
                ((next () s (resume s (* s 2))))
                (+ (St.next) (St.next))))
            (def (main (: n Int64))
              (+ (f1 n) (* 100 (f2 n))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 15011 Int64))
  (call   main (: 1 Int64)) (output (: 3003 Int64)))
