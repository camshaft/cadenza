(case "runtime Bytes ordering computes lexicographic over unsigned bytes after the blessing"
  (doc    "PR#1120 blessed a total order on Bytes (lexicographic over UNSIGNED bytes) — the flip of
           the old uniform decline. Three faces: plain lexicographic ([2,2]<[3,2] → 1), UNSIGNED at
           the sign boundary ([200,2]<[3,2] is FALSE — a signed-i8 byte compare says true since 200
           is -56 as i8), and shorter-prefix-is-less ([1]<[1,0] → 1). 101. Runtime operands defeat
           folding; guards the exact unsigned+prefix semantics the blessing names.")
  (input  (do
            (def (mk (: n Int64)) (Bytes.of (list (UInt8.wrap n) 2)))
            (def (main)
              (+ (* 100 (if (< (mk 2) (mk 3)) 1 0))
                 (+ (* 10 (if (< (mk 200) (mk 3)) 1 0))
                    (if (< (Bytes.of (list 1)) (Bytes.of (list 1 0))) 1 0))))
            (export main)))
  (output (: 101 Int64)))

(case "Bytes as a compound-ordering LEAF orders by the blessed unsigned-lexicographic walk"
  (doc    "The compound consequence of the Bytes blessing: a tuple with a Bytes component is now
           orderable (every component offers a total order), and the walk's Bytes leaf must use the
           same unsigned-lexicographic order the scalar compare does — [2,2]-tuple < [3,2]-tuple (10s)
           and the unsigned face 200-tuple < 3-tuple FALSE (1s) → 10. Was a uniform decline pre-#1120.")
  (input  (do
            (def (mk (: n Int64)) (tuple 1 (Bytes.of (list (UInt8.wrap n) 2))))
            (def (main)
              (+ (* 10 (if (< (mk 2) (mk 3)) 1 0))
                 (if (< (mk 200) (mk 3)) 1 0)))
            (export main)))
  (output (: 10 Int64)))

(case "rope-vs-flat Bytes compare content-canonically under the blessed order"
  (doc    "The heap-shape face: a 2-chunk CONCAT rope must compare Equal to its flat twin (chunk
           boundaries invisible to the blessed byte order) and Less than the flat form differing in
           the final byte — 11. A seam-oblivious pointer/chunk-shape compare, or a walk that ordered
           by first-chunk length before content, breaks a leg.")
  (input  (do
            (def (main (: x UInt8))
              (let ((rope (Bytes.concat (Bytes.of (list x)) (Bytes.of (list 20 30)))))
                (+ (* 10 (if (= (compare rope (Bytes.of (list x 20 30))) (Ordering.Equal)) 1 0))
                   (if (< rope (Bytes.of (list x 20 31))) 1 0))))
            (export main)))
  (call   main (: 10 UInt8)) (output (: 11 Int64)))
