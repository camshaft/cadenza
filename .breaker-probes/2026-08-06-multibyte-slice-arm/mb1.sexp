(case "mb1 the arm slices a crossed multibyte String at a scalar boundary — the slice window respects UTF-8"
  (input  (do
            (effect St (op cut (-> String Int64)))
            (def (main (: n Int64))
              (handle St 0
                ((cut (t) s
                  (resume (match (String.slice t 1 3)
                            ((Some w) (String.byte-len w))
                            ((None _u) -1))
                          s)))
                (St.cut (String.concat "a" "édc"))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 3 Int64)))
