(case "ch3 Option-Char from a COMPILE-TIME scalar-at beside performs folds"
  (input  (do
            (effect St (op bump (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St n
                ((bump (u) s (resume s (+ s 1))))
                (+ (St.bump)
                   (match (String.scalar-at "hello" 1)
                     ((Some c) (if (= c #\e) 1 0))
                     ((None _u) -1)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 6 Int64)))
