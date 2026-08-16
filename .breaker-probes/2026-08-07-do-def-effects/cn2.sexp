(case "cn2 the inner shadow's RESULT flows up into the outer computation — a let-bound region value scaled beside an outer draw"
  (input  (do
            (effect St (op next (-> Int64)))
            (def (main (: n Int64))
              (handle St n
                ((next () s (resume s (* s 2))))
                (let ((up (handle St 7
                            ((next () t (resume t (+ t 1))))
                            (+ (St.next) (St.next)))))
                  (+ (* up 10) (St.next)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 155 Int64))
  (call   main (: 0 Int64)) (output (: 150 Int64)))
