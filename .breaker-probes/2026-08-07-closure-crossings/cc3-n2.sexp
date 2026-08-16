(case "x" (input (do
  (effect St (op next (-> Int64)))
  (def (mk (: m Int64)) (fn ((: x Int64)) (* x m)))
  (def (main (: n Int64))
    (handle St n
      ((next () s (resume s (+ s 1))))
      (let ((f (mk (St.next))))
        (+ (f 10) (f (St.next))))))
  (export main)))
  (call main (: 2 Int64)) (output (: 50 Int64)))
