(example
  (id "run-length-encoding")
  (name "Run-length encoding")
  (theme "algorithms")
  (surface "sexpr")
  (source (do
  (def
    (at (: xs (List Int64)) (: i Int64))
    (match ((. List at) xs i) ((Some v) v) ((None) (trap "rle: index out of range"))))

  (def
    (go xs i n cur cnt acc)
    (if
      (= i n)
      ((. List push) acc #tuple(cur cnt))
      (let
        ((x (at xs i)))
        (if
          (= x cur)
          (go xs (+ i 1) n cur (+ cnt 1) acc)
          (go xs (+ i 1) n x 1 ((. List push) acc #tuple(cur cnt)))))))

  (def
    (main)
    (let
      ((xs #list(1 1 1 2 3 3)))
      (go xs 1 ((. List len) xs) (at xs 0) 1 (: #list() (List (Tuple Int64 Int64))))))

  (export main)))
  (expected (: #list(#tuple(1 3) #tuple(2 1) #tuple(3 2)) (List (Tuple Int64 Int64)))))
