(do
  (effect St (op next (-> Unit Int64)))
  (def (apply2 (: f (-> Int64 Int64)) (: x Int64)) (f (f x)))
  (def (main (: n Int64))
    (handle St n
      ((next (u) s (resume s (+ s 1))))
      (let ((base (St.next)))
        (apply2 (fn ((: x Int64)) (+ x base)) 100))))
  (export main))
