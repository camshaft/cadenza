(example
  (id "rule-110")
  (name "Rule 110 cellular automaton")
  (theme "algorithms")
  (surface "sexpr")
  (source (do
  (def (cell xs i) (match ((. List at) xs i) ((Some v) v) ((None) 0)))

  (def
    (rule l c r)
    (match (+ (+ (* l 4) (* c 2)) r) (0 0) (1 1) (2 1) (3 1) (4 0) (5 1) (6 1) (7 0) (_ 0)))

  (def
    (step-from xs i n acc)
    (if
      (= i n)
      acc
      (step-from
        xs
        (+ i 1)
        n
        ((. List push) acc (rule (cell xs (- i 1)) (cell xs i) (cell xs (+ i 1)))))))

  (def (step xs) (step-from xs 0 ((. List len) xs) #list()))

  (def (gens xs k) (if (= k 0) xs (gens (step xs) (- k 1))))

  (def (main) (gens #list(0 0 0 0 0 0 0 1) 4))

  (export main)))
  (expected (: #list(0 0 0 1 1 1 1 1) (List Int64))))
