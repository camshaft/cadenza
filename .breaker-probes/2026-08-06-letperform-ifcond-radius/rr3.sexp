(do
  (effect St (op bump (-> Unit Int64)))
  (def (f (: x Int64))
    (let ((ret (+ x (St.bump))))
      (* ret 2)))
  (def (main (: n Int64))
    (handle St 100
      ((bump (u) s (resume s (+ s 1))))
      (+ (f n) (f 2))))
  (export main))
