(example
  (id "reverse-a-list")
  (name "Reverse a list")
  (theme "basics")
  (surface "sexpr")
  (source (do
  (def
    (at (: xs (List Int64)) (: i Int64))
    (match ((. List at) xs i) ((Some v) v) ((None) (trap "reverse: index out of range"))))

  (def (rev xs i acc) (if (< i 0) acc (rev xs (- i 1) ((. List push) acc (at xs i)))))

  (def
    (main)
    (let ((xs #list(1 2 3 4 5))) (rev xs (- ((. List len) xs) 1) (: #list() (List Int64)))))

  (export main)))
  (expected (: #list(5 4 3 2 1) (List Int64))))
