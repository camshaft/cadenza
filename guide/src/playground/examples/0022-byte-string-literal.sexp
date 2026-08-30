(example
  (id "byte-string-literal")
  (name "Byte-string literal")
  (theme "data-and-collections")
  (surface "sexpr")
  (source (do
  (def
    (main)
    (let
      ((magic b"GIF89a"))
      (match
        ((. Bytes at) magic 0)
        ((Some first) #tuple(((. Bytes len) magic) first))
        ((None) (trap "byte-literal: unexpectedly empty")))))

  (export main)))
  (expected (: #tuple(6 71) (Tuple Int64 Int64))))
