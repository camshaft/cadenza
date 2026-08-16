(case "ds2 a THREE-deep nested slice re-bases at every level"
  (input  (do
            (def (main (: a Int64))
              (do
                (def v1 (Option.expect (String.slice "abcdefghij" a 9) "v1"))
                (def v2 (Option.expect (String.slice v1 1 6) "v2"))
                (def v3 (Option.expect (String.slice v2 2 4) "v3"))
                (+ (* 100 (if (= v3 "fg") 1 0))
                   (+ (* 10 (String.scalar-len v3))
                      (if (= v2 "defgh") 1 0)))))
            (export main)))
  (call   main (: 2 Int64)) (output (: 121 Int64)))
