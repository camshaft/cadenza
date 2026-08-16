(case "p" (input (do
  (effect St (op next (-> Int64)))
  (def (mk (: m Int64)) (fn ((: x Int64)) (* x m)))
  (def (main (: n Int64))
    (handle St n
      ((next () s (resume s (+ s 1))))
      (let ((f (mk (* n 3))))
        (+ (f 10) (f (St.next))))))
  (export main)))
  (call main (: 5 Int64)) (output (: 225 Int64)))
