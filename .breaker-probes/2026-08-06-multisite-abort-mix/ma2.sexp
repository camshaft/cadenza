(do
  (effect St (op sift (-> Int64 Int64)) (op bail (-> Unit Int64)))
  (def (main (: n Int64))
    (+ 1000
      (handle St 0
        ((sift (v) s (if (> v 10) (resume v (+ s 1)) (resume 0 s)))
         (bail (u) s (* s 10)))
        (+ (St.sift 20) (+ (St.bail) (St.sift 30))))))
  (export main))
