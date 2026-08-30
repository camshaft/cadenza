(example
  (id "bytes-map-key")
  (name "Bytes as a Map key")
  (theme "data-and-collections")
  (surface "sexpr")
  (source (do
  (def
    (bump (: m (Map Bytes Int64)) (: k Bytes))
    (match
      ((. Map lookup) m k)
      ((Some c) ((. Map insert) m k (+ c 1)))
      ((None) ((. Map insert) m k 1))))

  (def
    (main)
    (let
      ((red ((. String to-bytes) "red")) (blue ((. String to-bytes) "blue")))
      (let
        ((m (bump (bump (bump (bump ((. Map empty)) red) blue) red) red)))
        #tuple(((. Map lookup) m red) ((. Map lookup) m blue)))))

  (export main)))
  (expected (: #tuple((Some 3) (Some 1)) (Tuple (Option Int64) (Option Int64)))))
