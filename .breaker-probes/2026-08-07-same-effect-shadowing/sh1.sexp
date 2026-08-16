(case "sh1 a NESTED handler for the SAME effect shadows the outer — inner draws hit the inner state, the draw after hits the outer"
  (input  (do
            (effect St (op next (-> Int64)))
            (def (main (: n Int64))
              (handle St n
                ((next () s (resume s (+ s 1))))
                (+ (handle St 5
                     ((next () s (resume s (* s 10))))
                     (let ((a (St.next)))
                       (let ((b (St.next)))
                         (+ a b))))
                   (St.next))))
            (export main)))
  (call   main (: 100 Int64)) (output (: 155 Int64))
  (call   main (: 7 Int64)) (output (: 62 Int64)))
