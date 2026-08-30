(example
  (id "caesar-cipher-char")
  (name "Caesar cipher (char arithmetic)")
  (theme "algorithms")
  (surface "sexpr")
  (source (do
  (def (letter s) (match ((. String scalar-at) s 0) ((Some c) c) ((None) (trap "empty string"))))

  (def
    (shift c n)
    (let
      ((code ((. Char to-int) c)))
      (match
        ((. Char from-int) (+ 65 (% (+ (- code 65) n) 26)))
        ((Some r) r)
        ((None) (trap "caesar: bad code point")))))

  (def (main) #tuple((shift (letter "A") 3) (shift (letter "Y") 3)))

  (export main)))
  (expected (: #tuple(#\D #\B) (Tuple Char Char))))
