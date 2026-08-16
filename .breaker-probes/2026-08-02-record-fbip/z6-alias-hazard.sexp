(case "z6 a list-aliased record read after Record.with sees the ORIGINAL value (no in-place clobber)"
  (input  (do
            (def (bump-x (: r (Record (x Int64) (y Int64))))
              (Record.with r #"x" (+ (. r x) 1)))
            (def (main (: k Int64))
              (let ((seed (record (x k) (y 100))))
                (let ((alias (list seed)))
                  (let ((done (bump-x seed)))
                    (+ (. done x)
                       (* 100 (match (List.at alias 0) ((Some a) (. a x)) ((None _u) -1))))))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 304 Int64)))
