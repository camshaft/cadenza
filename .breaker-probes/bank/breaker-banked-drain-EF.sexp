(case "an insertion sort over Bytes elements uses the blessed order end-to-end"
  (doc    "The USER-ALGORITHM face of the #1120 Bytes blessing: an insort whose comparator is the
           blessed `<` sorts [3,1], [k=3], [200] into [3] < [3,1] < [200] (byte-lens 1,2,1 → 121) —
           the prefix rule places the extension after its prefix mid-sort, and the unsigned face
           keeps [200] last through TWO comparisons (each against a smaller-first-byte element).
           The scalar/enumeration pins check one comparison at a time; the sort composes the order
           through a recursive algorithm whose RESULT depends on every comparison agreeing — the
           consuming-code shape the blessing exists for (sorted key ranges).")
  (input  (do
            (def (insort (: x Bytes) (: xs (List Bytes)))
              (match xs
                ((list) (list x))
                ((list h .. t) (if (< x h) (List.concat (list x) xs)
                                          (List.concat (list h) (insort x t))))))
            (def (main (: k UInt8))
              (let ((sorted (insort (Bytes.of (list k))
                              (insort (Bytes.of (list 200))
                                (insort (Bytes.of (list 3 1)) (list))))))
                (+ (* 100 (Bytes.len (Option.expect (List.at sorted 0) "a")))
                   (+ (* 10 (Bytes.len (Option.expect (List.at sorted 1) "b")))
                      (Bytes.len (Option.expect (List.at sorted 2) "c"))))))
            (export main)))
  (call   main (: 3 UInt8)) (output (: 121 Int64)))
