(example
  (id "value-encode-canonical-bytes")
  (name "Canonical binary encoding (Value.encode)")
  (theme "data-and-collections")
  (surface "sexpr")
  (source (do
  (def (size v) ((. Bytes len) ((. Value encode) v)))

  (def (main) #tuple((size #record((= x 3) (= y 4))) (size (Some 7)) (size #list(1 2 3))))

  (export main)))
  (expected (: (tuple 102 61 66) (Tuple Int64 Int64 Int64))))
