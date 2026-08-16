(do
  (effect St (op sift (-> Int64 Int64)))
  (def (classify (: x Int64)) (Some (+ x 1)))
  (def (main (: n Int64))
    (handle St 0
      ((sift (v) s (if (> v 10) (resume v (+ s 1)) (resume 0 s))))
      (match (classify (St.sift 20))
        ((Option.Some r) (+ r (St.sift n)))
        ((Option.None _u) -1))))
  (export main))
