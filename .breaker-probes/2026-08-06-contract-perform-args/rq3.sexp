(do
  (effect St (op next (-> Unit Int64)))
  (@ (ensures (> ret 0)) (def (g (: x Int64)) (* x 2)))
  (def (main (: n Int64))
    (handle St 21
      ((next (u) s (resume s (+ s 1))))
      (let ((v (St.next)))
        (if (> v 5) (g v) (+ (g v) (g 1))))))
  (export main))
