(example
  (id "quicksort")
  (name "Quicksort")
  (theme "algorithms")
  (surface "sexpr")
  (source (do
  (def
    (at (: xs (List Int64)) (: i Int64))
    (match ((. List at) xs i) ((Some v) v) ((None) (trap "qsort: index out of range"))))

  (def
    (part xs i n pivot lows highs)
    (if
      (= i n)
      #tuple(lows highs)
      (let
        ((x (at xs i)))
        (if
          (< x pivot)
          (part xs (+ i 1) n pivot ((. List push) lows x) highs)
          (part xs (+ i 1) n pivot lows ((. List push) highs x))))))

  (def
    (qsort (: xs (List Int64)))
    (if
      (< ((. List len) xs) 2)
      xs
      (let
        ((pivot (at xs 0)))
        (match
          (part xs 1 ((. List len) xs) pivot (: #list() (List Int64)) (: #list() (List Int64)))
          (#tuple(lows highs)
            ((. List concat) ((. List concat) (qsort lows) #list(pivot)) (qsort highs)))))))

  (def (main) (qsort #list(5 3 8 1 9 2 7 4 6)))

  (export main)))
  (expected (: #list(1 2 3 4 5 6 7 8 9) (List Int64))))
