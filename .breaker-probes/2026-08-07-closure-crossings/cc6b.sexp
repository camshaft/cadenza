(case "cc6b a closure bound OUTSIDE the outer handle applied inside a nested SHADOW region and after it — capture crosses both boundaries"
  (input  (do
            (effect St (op next (-> Int64)))
            (def (main (: n Int64))
              (let ((f (fn ((: x Int64)) (+ x (* n 100)))))
                (handle St n
                  ((next () s (resume s (+ s 1))))
                  (+ (handle St 50
                       ((next () t (resume t (* t 2))))
                       (f (St.next)))
                     (f (St.next))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 1055 Int64))
  (call   main (: 0 Int64)) (output (: 50 Int64)))
