(example
  (id "matrix-transpose")
  (name "Matrix transpose")
  (theme "data-and-collections")
  (surface "sexpr")
  (source (do
  (def
    (elem (: m (List (List Int64))) (: i Int64) (: j Int64))
    (match
      ((. List at) m i)
      ((Some r)
        (match
          ((. List at) r j)
          ((Some v) v)
          ((None) (trap "transpose: column index out of range"))))
      ((None) (trap "transpose: row index out of range"))))

  (def
    (col m j i rows acc)
    (if (= i rows) acc (col m j (+ i 1) rows ((. List push) acc (elem m i j)))))

  (def
    (go m j cols rows acc)
    (if
      (= j cols)
      acc
      (go m (+ j 1) cols rows ((. List push) acc (col m j 0 rows (: #list() (List Int64)))))))

  (def
    (main)
    (let ((m #list(#list(1 2 3) #list(4 5 6)))) (go m 0 3 2 (: #list() (List (List Int64))))))

  (export main)))
  (expected (: #list(#list(1 4) #list(2 5) #list(3 6)) (List (List Int64)))))
