(example
  (id "value-encode-determinism")
  (name "Structural encoding is deterministic (Value.encode)")
  (theme "data-and-collections")
  (surface "sexpr")
  (source (do
  (def
    (main)
    (let
      ((ba ((. Value encode) #record((= x 3) (= y 4))))
        (bb ((. Value encode) #record((= x 3) (= y 4))))
        (bc ((. Value encode) #record((= x 3) (= y 5)))))
      #tuple(((. Bytes len) ba) (= ba bb) (= ba bc))))

  (export main)))
  (expected (: #tuple(102 true false) (Tuple Int64 Bool Bool))))
