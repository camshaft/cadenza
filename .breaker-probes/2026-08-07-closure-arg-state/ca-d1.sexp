(case "d1" (input (do
  (effect St (op next (-> Int64)))
  (def (main (: n Int64))
    (handle St n
      ((next () s (resume s (+ s 1))))
      (let ((a (St.next)))
        (let ((f (fn ((: x Int64)) (* a x))))
          (+ (f (St.next)) (f 10))))))
  (export main)))
  (call main (: 5 Int64)) (output (: 80 Int64)))
