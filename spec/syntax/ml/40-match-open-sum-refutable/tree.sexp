(do
  (type O (Wrap Int64) .. r)

  (def (f o) (match o ((Wrap x) x))))
