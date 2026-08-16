(do
  (effect St (op sift (-> Int64 Int64)) (op reset (-> Unit Int64)))
  (def (main (: n Int64))
    (handle St 0
      ((sift (v) s (resume v (+ s 1)))
       (reset (u) s (resume s 100)))
      (+ (St.sift 20) (+ (St.reset) (St.sift 30)))))
  (export main))
