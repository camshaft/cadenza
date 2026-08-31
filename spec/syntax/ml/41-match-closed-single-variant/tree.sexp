(do
  (type C (Wrap Int64))

  (def (f o) (match o ((Wrap x) x))))
