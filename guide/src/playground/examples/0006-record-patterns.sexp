(example
  (id "record-patterns")
  (name "Record patterns (match on fields)")
  (theme "basics")
  (surface "sexpr")
  (source (do
  (def
    (score p)
    (match p (#record((= x 0) (= y 0)) 0) (#record((= x xv) (= y yv)) (+ (* xv xv) (* yv yv)))))

  (def (main) (score #record((= x 3) (= y 4))))

  (export main)))
  (expected 25))
