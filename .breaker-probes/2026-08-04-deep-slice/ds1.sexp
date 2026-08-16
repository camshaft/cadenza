(case "ds1 nested slice over a DEPTH-2 rope crossing both seams with multibyte leaves"
  (input  (do
            (def (main (: a Int64))
              (do
                (def s (String.concat "aé" (String.concat "日x" "y😀")))
                (def outer (Option.expect (String.slice s a 5) "outer"))
                (def inner (Option.expect (String.slice outer 1 3) "inner"))
                (+ (* 100 (String.byte-len inner))
                   (+ (* 10 (String.scalar-len inner))
                      (match (String.at inner 1) ((Some c) (if (= c "x") 4 0)) ((None _u) -1))))))
            (export main)))
  (call   main (: 1 Int64)) (output (: 424 Int64)))
