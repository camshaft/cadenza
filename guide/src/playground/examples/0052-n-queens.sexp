(example
  (id "n-queens")
  (name "N-queens (count solutions)")
  (theme "algorithms")
  (surface "sexpr")
  (source (do
  (def
    (at (: xs (List Int64)) (: i Int64))
    (match ((. List at) xs i) ((Some v) v) ((None) (trap "queens: placed[i] out of range"))))

  (def (adiff a b) (if (> a b) (- a b) (- b a)))

  (def
    (safe placed col row i)
    (if
      (= i row)
      true
      (let
        ((pc (at placed i)))
        (if
          (= pc col)
          false
          (if (= (adiff pc col) (adiff i row)) false (safe placed col row (+ i 1)))))))

  (def
    (try-cols n placed row col acc)
    (if
      (= col n)
      acc
      (try-cols
        n
        placed
        row
        (+ col 1)
        (if (safe placed col row 0) (+ acc (solve n ((. List push) placed col) (+ row 1))) acc))))

  (def (solve n placed row) (if (= row n) 1 (try-cols n placed row 0 0)))

  (def (main) (solve 8 (: #list() (List Int64)) 0))

  (export main)))
  (expected 92))
