(case "r1" (input (do
  (effect St (op next (-> Int64)))
  (def (apply2 (: g (-> Int64 Int64)))
    (+ (g 1) (g 2)))
  (def (main (: n Int64))
    (handle St n
      ((next () s (resume s (+ s 1))))
      (apply2 (let ((a (St.next))) (fn ((: x Int64)) (* a x))))))
  (export main)))
  (call main (: 5 Int64)) (output (: 15 Int64)))
