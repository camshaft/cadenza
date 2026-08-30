(example
  (id "string-slice-date-fields")
  (name "Substring slicing (ISO date fields)")
  (theme "data-and-collections")
  (surface "sexpr")
  (source (do
  (def (field s lo hi) (match ((. String slice) s lo hi) ((Some part) part) ((None) "")))

  (def (main) (let ((d "2026-08-14")) #tuple((field d 0 4) (field d 5 7) (field d 8 10))))

  (export main)))
  (expected (: #tuple("2026" "08" "14") (Tuple String String String))))
