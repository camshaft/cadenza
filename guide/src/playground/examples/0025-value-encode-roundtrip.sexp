(example
  (id "value-encode-roundtrip")
  (name "Encode / decode round-trip (Value)")
  (theme "data-and-collections")
  (surface "sexpr")
  (source (do
  (type Point (Mk (Record (: x Int64) (: y Int64))))

  (def
    (main)
    (let
      ((p ((. Point Mk) #record((= x 3) (= y 4)))))
      (let
        ((bytes ((. Value encode) p)))
        (match
          (: ((. Value decode) bytes) (Option Point))
          ((Some ((. Point Mk) r)) #tuple(((. Bytes len) bytes) (+ (. r x) (. r y))))
          ((None) #tuple(((. Bytes len) bytes) 0))))))

  (export main)))
  (expected (: (tuple 73 7) (Tuple Int64 Int64))))
