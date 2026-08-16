(case "an1 explicitly ASCRIBED draws — (: (E.next) Int64) in let and argument positions changes nothing observable"
  (input  (do
            (effect E (op next (-> Int64)) (op probe (-> Int64)))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 1)))
                 (probe () s (resume s s)))
                (let ((a (: (E.next) Int64)))
                  (+ (* 10 (+ a (: (E.next) Int64))) (- (E.probe) n)))))
            (export main)))
  (call   main (: 4 Int64)) (output (: 92 Int64))
  (call   main (: 0 Int64)) (output (: 12 Int64))
  (call   main (: -3 Int64)) (output (: -48 Int64)))
