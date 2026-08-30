(example
  (id "unicode-nfc-normalization")
  (name "Unicode NFC normalization")
  (theme "data-and-collections")
  (surface "sexpr")
  (source (do
  (def (accent) ((. String from-bytes) ((. Bytes of) #list(204 129))))

  (def
    (main)
    (match
      (accent)
      ((Some acc)
        (let
          ((composed ((. String concat) "e" acc)))
          #tuple(((. String scalar-len) composed) ((. String byte-len) composed))))
      ((None) (trap "nfc: invalid accent bytes"))))

  (export main)))
  (expected (: (tuple 1 2) (Tuple Int64 Int64))))
