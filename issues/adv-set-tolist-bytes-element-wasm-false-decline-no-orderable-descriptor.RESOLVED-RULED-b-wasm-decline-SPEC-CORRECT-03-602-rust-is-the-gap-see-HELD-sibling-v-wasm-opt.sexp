; FINDING (breaker, 2026-07-27): Set.to-list over BYTES elements — wasm FALSE-DECLINES
; ("Set.to-list element shape has no orderable descriptor") while BOTH rust targets compute the
; lexicographic order and PASS. The exact recurrence of the pinned tuple/record/float wasm<->rust
; divergence family (19-sets :1466-:1510): the orderable-descriptor guard/sort lacks a BYTES arm
; on the wasm side, while rust's total order already covers it. Same class as the
; symbol-set-map-to-list shape_of trap — the guard admits the type, the wasm sort descriptor
; doesn't. ROPE IRRELEVANT: a flat-only Bytes set false-declines identically; Set.len/contains
; over Bytes work on wasm (only the to-list ORDER surface is missing).
;
; Expected order: content lexicographic, shorter prefix first — [1,2] < [1,2,3] < [5]
; (verified on rust + rust-async: 231 via per-position Bytes.len digits; n=3 builds [1,2,3] as a
; rope so the compare also crosses a concat seam).
(case "Set.to-list orders a set of BYTES elements lexicographically with a shorter prefix first"
  (input  (do
        (def (main (: n UInt8))
          (do
            (def r (Bytes.concat (Bytes.of (list 1 2)) (Bytes.of (list n))))
            (def s (Set.of (list (Bytes.of (list 5)) r (Bytes.of (list 1 2)))))
            (def xs (Set.to-list s))
            (def (lat (: i Int64)) (Bytes.len (Option.expect (List.at xs i) "in")))
            (+ (* 100 (lat 0)) (+ (* 10 (lat 1)) (lat 2)))))
        (export main)))
  (call   main (: 3 UInt8)) (output (: 231 Int64)))
