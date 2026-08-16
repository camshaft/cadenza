(case "Set.to-list over Bytes elements enumerates in the blessed unsigned-lexicographic order"
  (doc    "The CANONICAL-ENUMERATION consequence of the #1120 Bytes blessing: a Set of {[200], [x=3],
           [3,1]} must enumerate [3], [3,1], [200] — element-wise blessed order with the prefix rule
           ([3] before its extension [3,1]) and the UNSIGNED face ([200] LAST; a signed byte order
           sorts it first as -56). Byte-lens read 1,2,1 → 121. The collection-order face of the
           blessing (the scalar/compound/rope compare faces are pinned beside it) — Set.to-list's
           order for Bytes elements was undefined-by-decline before #1120.")
  (input  (do
            (def (main (: x UInt8))
              (let ((s (Set.of (list (Bytes.of (list 200))
                                     (Bytes.of (list x))
                                     (Bytes.of (list 3 1))))))
                (+ (* 100 (Bytes.len (Option.expect (List.at (Set.to-list s) 0) "a")))
                   (+ (* 10 (Bytes.len (Option.expect (List.at (Set.to-list s) 1) "b")))
                      (Bytes.len (Option.expect (List.at (Set.to-list s) 2) "c"))))))
            (export main)))
  (call   main (: 3 UInt8)) (output (: 121 Int64)))
