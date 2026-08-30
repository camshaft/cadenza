(example
  (id "tuple")
  (name "A tuple")
  (theme "basics")
  (surface "sexpr")
  (source (do
  (def (main) #tuple(1 2 3))

  (export main)))
  (expected (: #tuple(1 2 3) (Tuple Int64 Int64 Int64))))
