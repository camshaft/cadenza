(case "a multibyte string round-trips through to-bytes and from-bytes preserving scalar content"
  (doc    "The MULTIBYTE + structural-= face of the encode-decode round-trip (the pinned round-trip
           :2671 is ASCII \"hi\" and measures LENGTH only): a rope-built `café!` (5 scalars, é = 2
           bytes = 6 total) to-bytes→from-bytes must yield `Some s2` with s2 `=` the original (the
           composed bytes are already NFC, so the round-trip preserves — contrast the decomposed-input
           non-normalization pin :2665), scalar-len 5, byte-len 6 → 156. The to-bytes is read ONCE
           (feeding from-bytes), so this stays clear of the adv-54 multi-read view bug and pins the
           inverse-round-trip on a genuine multibyte payload by content, not just size.")
  (input  (do
            (def (main (: k Int64))
              (let ((s (String.concat "café" "!")))
                (match (String.from-bytes (String.to-bytes s))
                  ((Some s2) (+ (* 100 (if (= s s2) 1 0))
                                (+ (* 10 (String.scalar-len s2)) (String.byte-len s2))))
                  ((None u) -1))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 156 Int64)))
