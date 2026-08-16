(do
  (effect St (op next (-> Unit Int64)))
  (@ (requires (> x 0)) (@ (ensures (> ret 0)) (def (h (: x Int64)) (* x 2))))
  (def (main (: n Int64))
    (handle St 21
      ((next (u) s (resume s (+ s 1))))
      (+ (h (St.next)) (h (St.next)))))
  (export main))
