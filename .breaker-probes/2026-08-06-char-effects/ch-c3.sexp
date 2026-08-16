(do
  (effect St (op bump (-> Unit Int64)))
  (def (main (: n Int64))
    (handle St n
      ((bump (u) s (resume s (+ s 1))))
      (+ (St.bump)
         (match (String.scalar-at "hello" 1)
           ((Some c) (if (= c #\e) 1 0))
           ((None _u) -1)))))
  (export main))
