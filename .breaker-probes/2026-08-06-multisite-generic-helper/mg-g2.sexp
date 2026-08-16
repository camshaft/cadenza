(do
  (effect St (op sift (-> Int64 Int64)))
  (def (both f a b) (+ (f a) (f b)))
  (def (main (: n Int64))
    (handle St 0
      ((sift (v) s (resume v (+ s 1))))
      (both (fn ((: x Int64)) (St.sift x)) 20 n)))
  (export main))
