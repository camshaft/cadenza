(example
  (id "count-vowels")
  (name "Count vowels")
  (theme "algorithms")
  (surface "sexpr")
  (source (do
  (def
    (is-vowel b)
    (if
      (= b 97)
      true
      (if (= b 101) true (if (= b 105) true (if (= b 111) true (if (= b 117) true false))))))

  (def
    (byte-at bs i)
    (match ((. Bytes at) bs i) ((Some b) b) ((None) (trap "count-vowels: byte index out of range"))))

  (def
    (go bs i n acc)
    (if (= i n) acc (go bs (+ i 1) n (if (is-vowel (byte-at bs i)) (+ acc 1) acc))))

  (def (count-vowels s) (let ((bs ((. String to-bytes) s))) (go bs 0 ((. Bytes len) bs) 0)))

  (def (main) (count-vowels "education"))

  (export main)))
  (expected 5))
