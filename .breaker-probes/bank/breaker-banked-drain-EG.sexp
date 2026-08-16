(case "an insertion sort over MULTIBYTE strings uses unsigned byte order end-to-end"
  (doc    "The multibyte face of String order composed through a sort: \"é\" (0xC3A9, 2 bytes) must
           sort AFTER \"z\" (0x7A) — the unsigned byte order the scalar pin blesses (\"z\"<\"é\",
           13-strings:78), here driving a recursive insort where é passes TWO comparisons to reach
           the last slot (byte-lens 1,1,2 → 112). A signed-i8 lead-byte compare sorts é FIRST (0xC3
           = -61); a scalar-VALUE order happens to agree here but the pinned semantics is bytes. The
           String companion of the Bytes-sort pin and the tuple-insort (:535 — compound elements;
           this one's elements are heap STRINGS with a multibyte member).")
  (input  (do
            (def (insort (: x String) (: xs (List String)))
              (match xs
                ((list) (list x))
                ((list h .. t) (if (< x h) (List.concat (list x) xs)
                                          (List.concat (list h) (insort x t))))))
            (def (main (: k Int64))
              (let ((sorted (insort "é" (insort "z" (insort "a" (list))))))
                (+ (* 100 (String.byte-len (Option.expect (List.at sorted 0) "a")))
                   (+ (* 10 (String.byte-len (Option.expect (List.at sorted 1) "b")))
                      (String.byte-len (Option.expect (List.at sorted 2) "c"))))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 112 Int64)))
