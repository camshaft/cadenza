(do
  (type Box (Box Int64))

  (def (f p) (match p ((tuple a (Box x)) (+ a x)))))
