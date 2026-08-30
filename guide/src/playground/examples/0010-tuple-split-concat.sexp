(example
  (id "tuple-split-concat")
  (name "Splitting & joining tuples")
  (theme "basics")
  (surface "sexpr")
  (source (do
  (def
    (main)
    (let
      ((parts ((. Tuple split-at) #tuple(1 2 3 4 5) 2)))
      (match parts (#tuple(head tail) #tuple(head tail ((. Tuple concat) head tail))))))

  (export main)))
  (expected (: #tuple(#tuple(1 2) #tuple(3 4 5) #tuple(1 2 3 4 5)) (Tuple (Tuple Int64 Int64) (Tuple Int64 Int64 Int64) (Tuple Int64 Int64 Int64 Int64 Int64)))))
