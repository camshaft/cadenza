(case "sg3 a ONE-SHOT string lock — the arm string-compares op arg vs state, consuming the key on the first match"
  (input  (do
            (effect Lock (op try (-> String Int64)))
            (def (main (: n Int64))
              (handle Lock "key"
                ((try (w) s (if (= w s) (resume 1 "used") (resume 0 s))))
                (+ (Lock.try (if (> n 3) "key" "nope"))
                   (+ (* 10 (Lock.try "key")) (* 100 (Lock.try "used"))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 101 Int64))
  (call   main (: 0 Int64)) (output (: 110 Int64)))
