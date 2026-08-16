(case "a List.update lands correctly inside a concat-built RELAXED trie past the seam"
  (doc    "Update × the relaxed-node rep: `List.concat` of two 40-element halves leaves RELAXED
           interior nodes with a size table (the :3329/:3346 shape-observability family pins that
           rep divergence), and `List.update xs 45 -7` must navigate the size table PAST the seam to
           the second half's index 5 — read back updated (-7000), ORIGINAL intact (50 — persistence
           through the relaxed node's path-copy), seam-neighbor untouched (4) → -6946. The pinned
           update-at-depth pins (BS, :12861) drive PUSH-built strict tries; the size-table descent of
           a relaxed trie is a different index-resolution path (a strict-radix shortcut lands on the
           wrong element past a seam).")
  (input  (do
            (def (build (: i Int64) (: n Int64) acc)
              (if (= i n) acc (build (+ i 1) n (List.push acc i))))
            (def (main (: k Int64))
              (let ((xs (List.concat (build 0 40 (list)) (build 0 40 (list)))))
                (let ((ys (List.update xs 45 -7)))
                  (+ (* 1000 (Option.expect (List.at ys 45) "u"))
                     (+ (* 10 (Option.expect (List.at xs 45) "o"))
                        (Option.expect (List.at ys 44) "s"))))))
            (export main)))
  (call   main (: 0 Int64)) (output (: -6946 Int64)))
