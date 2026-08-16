(case "rt1 String→Bytes→compact→String round-trip over a multibyte ROPE preserves content"
  (input  (do
            (def (main (: k Int64))
              (do
                (def s (String.concat "aé" (String.concat "日" (if (> k 0) "x😀" "y"))))
                (def b (Bytes.compact (String.to-bytes s)))
                (match (String.from-bytes b)
                  ((Some s2) (+ (* 10 (if (= s2 s) 1 0)) (String.scalar-len s2)))
                  ((None _u) -1))))
            (export main)))
  (call   main (: 1 Int64)) (output (: 15 Int64))
  (call   main (: 0 Int64)) (output (: 14 Int64)))
