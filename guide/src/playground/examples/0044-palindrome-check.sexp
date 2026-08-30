(example
  (id "palindrome-check")
  (name "Palindrome check")
  (theme "algorithms")
  (surface "sexpr")
  (source (do
  (def
    (at bs i)
    (match ((. Bytes at) bs i) ((Some b) b) ((None) (trap "palindrome: byte index out of range"))))

  (def (pal bs i j) (if (>= i j) true (if (= (at bs i) (at bs j)) (pal bs (+ i 1) (- j 1)) false)))

  (def (main) (let ((bs ((. String to-bytes) "racecar"))) (pal bs 0 (- ((. Bytes len) bs) 1))))

  (export main)))
  (expected true))
