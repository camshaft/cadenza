(example
  (id "list-update-functional")
  (name "Functional list update")
  (theme "basics")
  (surface "sexpr")
  (source (do
  (def (set-at xs i v) ((. List update) xs i v))

  (def (main) (let ((xs #list(10 20 30 40 50))) #tuple((set-at xs 0 99) (set-at xs 4 99) xs)))

  (export main)))
  (expected (: #tuple(#list(99 20 30 40 50) #list(10 20 30 40 99) #list(10 20 30 40 50)) (Tuple (List Int64) (List Int64) (List Int64)))))
