(do
  (effect St (op pick (-> Int64 Char)))
  (def (main (: n Int64))
    (handle St "hello"
      ((pick (i) s (resume (match (String.scalar-at s i) ((Some c) c) ((None _u) #\x)) s)))
      (match (St.pick n)
        (#\e 1)
        (_ 0))))
  (export main))
