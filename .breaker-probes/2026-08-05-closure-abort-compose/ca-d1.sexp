(do
  (effect St (op halt (-> Unit Int64)))
  (def (main (: n Int64))
    (let ((p (handle St n
               ((halt (u) s (tuple (fn ((: x Int64)) (+ x s)) s)))
               (do (St.halt)
                   (tuple (fn ((: x Int64)) x) -999)))))
      (match p
        ((tuple g w) (+ (g 100) w)))))
  (export main))
