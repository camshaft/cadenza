(case "st1 a handle expression as ONE operand of an enclosing pure sum — the pure operand and the handled region compose"
  (input  (do
            (effect St (op next (-> Int64)))
            (def (main (: n Int64))
              (+ (* 1000 n)
                 (handle St n
                   ((next () s (resume s (+ s 1))))
                   (+ (St.next) (St.next)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 5011 Int64))
  (call   main (: 0 Int64)) (output (: 1 Int64)))
