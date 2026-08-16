(do
  (effect St (op sift (-> Int64 Int64)))
  (def (classify (: x Int64))
    (do
      (def v (try (Some x)))
      (Some (+ v 1))))
  (def (main (: n Int64))
    (handle St 0
      ((sift (v) s (resume v (+ s 1))))
      (match (classify (St.sift 20))
        ((Option.Some r) (+ r (St.sift n)))
        ((Option.None _u) -1))))
  (export main))
