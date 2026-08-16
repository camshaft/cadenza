(case "sy13 a symbol interned from a TORN-then-repaired byte round-trip stays canonical"
  (input  (do
            (def (main (: k Int64))
              (let ((s (String.concat "aé" "b")))
                (match (String.from-bytes (String.to-bytes s))
                  ((Some back)
                    (let ((sym1 (Symbol.of s))
                          (sym2 (Symbol.of back)))
                      (+ (if (= sym1 sym2) 1 0)
                         (* 10 (String.byte-len (Symbol.to-string sym1))))))
                  ((None _u) -1))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 41 Int64)))
