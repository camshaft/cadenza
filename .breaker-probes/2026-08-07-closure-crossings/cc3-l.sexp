(case "l" (input (do
  (effect St (op next (-> Int64)))
  (def (mk (: m Int64)) (fn ((: x Int64)) (* x m)))
  (def (main (: n Int64))
    (handle St n
      ((next () s (resume s (+ s 1))))
      (let ((d (St.next)))
        (let ((f (mk d)))
          (+ (f 10) (f (St.next)))))))
  (export main)))
  (call main (: 5 Int64)) (output (: 80 Int64)))
