(do (def (b (: n Int64)) (bin (bits ((. (UInt 3) wrap) n) 3) (bits ((. (UInt 1) wrap) 1) 1) (bits ((. (UInt 4) wrap) n) 4)))
    (def (at (: x Bytes) (: i Int64)) (match (Bytes.at x i) ((Some v) v) ((None _u) -1)))
    (def (main (: n Int64)) (do (def x (b n)) (+ (* 100 (Bytes.len x)) (at x 0))))
    (export main))
