(do
  (effect St (op next (-> Unit Int64)))
  (def (twice (: f (-> Int64 Int64)) (: x Int64)) (+ (f x) (f x)))
  (def (main (: n Int64))
    (handle St n
      ((next (u) s (resume s (+ s 1))))
      (twice (fn ((: x Int64)) (+ x (St.next))) 100)))
  (export main))
