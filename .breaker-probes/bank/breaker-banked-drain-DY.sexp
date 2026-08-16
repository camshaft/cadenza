(case "branch-SELECTED operand values encode into bin segments per-branch"
  (doc    "The IF-JOINED operand face of bin construction: the u16 segment's operand is `(if b 258
           772)` — a runtime branch join feeding the encoder — and the byte image must reflect
           WHICHEVER value the branch chose: [1,2] (258=0x0102 BE) or [3,4] (772=0x0304), read back
           by per-byte decode with the trailing u8 as a position anchor → 10209/30409. An encoder
           that const-folded the segment to one branch's bytes (or materialized the join wrong at
           the segment width) breaks one call. The pinned runtime-operand encode pins use PARAMS;
           the join-materialized operand was unpinned.")
  (input  (do
            (def (main (: b Bool))
              (match (bin (u16 (if b 258 772)) (u8 9))
                ((bin (u8 hi) (u8 lo) (u8 t)) (Int64.of (+ (* 10000 hi) (+ (* 100 lo) t))))
                (_ -1)))
            (export main)))
  (call   main (: true Bool)) (output (: 10209 Int64))
  (call   main (: false Bool)) (output (: 30409 Int64)))
