(case "ce1 a crossed rope String compares EQUAL to an arm-local flat literal — content equality over the marshal"
  (input  (do
            (effect St (op check (-> String Int64)))
            (def (main (: n Int64))
              (handle St 0
                ((check (t) s
                  (resume (+ (* 100 (if (= t "abcde") 1 0))
                             (+ (* 10 (if (< t "abd") 1 0))
                                (if (< "abd" t) 1 0)))
                          s)))
                (St.check (String.concat "ab" (if (> n 0) "cde" "z")))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 110 Int64)))
