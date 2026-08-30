(example
  (id "binary-search")
  (name "Binary search")
  (theme "algorithms")
  (surface "sexpr")
  (source (do
  (def
    (at (: xs (List Int64)) (: i Int64))
    (match ((. List at) xs i) ((Some v) v) ((None) (trap "bsearch: index out of range"))))

  (def
    (go xs target lo hi)
    (if
      (> lo hi)
      (: (None) (Option Int64))
      (let
        ((mid (/ (+ lo hi) 2)) (v (at xs mid)))
        (if
          (= v target)
          (Some mid)
          (if (< v target) (go xs target (+ mid 1) hi) (go xs target lo (- mid 1)))))))

  (def (bsearch (: xs (List Int64)) (: target Int64)) (go xs target 0 (- ((. List len) xs) 1)))

  (def (main) (let ((xs #list(1 3 5 7 9 11 13 15 17 19))) #tuple((bsearch xs 11) (bsearch xs 8))))

  (export main)))
  (expected (: #tuple((Some 5) (None unit)) (Tuple (Option Int64) (Option Int64)))))
