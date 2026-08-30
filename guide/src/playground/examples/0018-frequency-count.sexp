(example
  (id "frequency-count")
  (name "Frequency count (fold into a Map)")
  (theme "data-and-collections")
  (surface "sexpr")
  (source (do
  (def
    (bump mp k)
    (match
      ((. Map lookup) mp k)
      ((Some c) ((. Map insert) mp k (+ c 1)))
      ((None) ((. Map insert) mp k 1))))

  (def
    (tally xs i n mp)
    (if
      (= i n)
      mp
      (match ((. List at) xs i) ((Some x) (tally xs (+ i 1) n (bump mp x))) ((None) mp))))

  (def
    (main)
    (let ((xs #list(3 1 3 3 1 2))) ((. Map to-list) (tally xs 0 ((. List len) xs) ((. Map empty))))))

  (export main)))
  (expected (: #list(#tuple(1 2) #tuple(2 1) #tuple(3 3)) (List (Tuple Int64 Int64)))))
