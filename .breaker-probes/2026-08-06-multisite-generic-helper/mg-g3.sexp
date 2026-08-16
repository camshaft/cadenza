(do
  (effect St (op sift (-> Int64 Int64)))
  (def (main (: n Int64))
    (handle St 0
      ((sift (v) s (if (> v 10) (resume v (+ s 1)) (resume 0 s))))
      (let ((f (fn ((: x Int64)) (St.sift x))))
        (+ (f 20) (f n)))))
  (export main))
