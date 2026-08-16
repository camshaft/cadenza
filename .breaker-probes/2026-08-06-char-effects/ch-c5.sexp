(do
  (def (main (: n Int64))
    (match (String.scalar-at "hello" n)
      ((Some c) (if (= c #\e) 1 0))
      ((None _u) -1)))
  (export main))
