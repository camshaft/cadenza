(do
  (def (main)
    (do
      (match (record (x 3)) ((record (z a)) a))
      (. (record (y 4)) z)))
  (export main))
