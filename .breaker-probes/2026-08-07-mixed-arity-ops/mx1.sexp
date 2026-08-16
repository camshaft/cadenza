(case "mx1 a THREE-arg op mixing Int64/String/Bool — one arm consumes all three kinds beside the live state"
  (input  (do
            (effect E (op mix (-> Int64 String Bool Int64)))
            (def (main (: n Int64))
              (handle E n
                ((mix (k w f) s (resume (+ (* (if f 10 1) k) (+ (String.byte-len w) s)) (+ s 1))))
                (+ (E.mix 3 "ab" true) (E.mix 4 "xyz" false))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 50 Int64))
  (call   main (: 0 Int64)) (output (: 40 Int64)))
