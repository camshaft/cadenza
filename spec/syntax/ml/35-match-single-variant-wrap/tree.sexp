(do
  (type Wrapper (Wrap Int64))

  (def (f w) (match w ((Wrap x) x))))
