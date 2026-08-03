; adv-54 (breaker tick 1117) — WASM-ONLY SOUNDNESS MISCOMPILE: a let-bound (String.to-bytes <view>)
; where <view> is a String.slice (a rope VIEW, not a fresh owned string), read TWICE via Bytes.at,
; returns ZEROS on the second+ read (or an out-of-bounds TRAP) on wasm — rust/rust-async compute
; correctly. The to-bytes buffer is CONSUMED by the first read.
;
; Observed (trunk c1c5efcca):
;   (let ((b (String.to-bytes tail)))          ; tail = (String.slice s 3 5), s multibyte
;     (+ (Bytes.at b 0) (Bytes.at b 1)))        ; wasm: 100 (only b0; b1 reads 0); rust: 295
;   Bytes.len alone: OK. ONE Bytes.at: OK. TWO reads: b[0] right, b[1..] ZERO. len+at: len ok, at 0.
;   Inline (no let) with 3 reads: OUT-OF-BOUNDS TRAP (original probe-dy).
;
; SHRINK (tick 1117, /tmp/breaker-shrink3):
;   t1/t2 Bytes.len of the slice's to-bytes          -> OK (len is right)
;   t4/t5/t6 ONE Bytes.at (inline or let)            -> OK
;   t7 TWO Bytes.at of let-bound to-bytes, MULTIBYTE slice -> 100 not 295  <- BUG (wasm)
;   t8 len + one at, multibyte slice                 -> 300 not 400 (the at reads 0)
;   t10 TWO Bytes.at, ASCII slice                    -> OK (201) — MULTIBYTE is essential
;   t11 TWO Bytes.at of to-bytes of a CONCAT (no slice), multibyte -> OK (295)
;   t9 whole-string (no slice) double read           -> OK
;   t12 helper-returned slice, double read           -> 122 not 317  <- BUG (reproduces)
; Trigger = to-bytes of a String.SLICE (a non-owned rope VIEW) + MULTIBYTE content + the resulting
; Bytes read MORE THAN ONCE. The to-bytes lowering for a sliced/view source apparently produces a
; buffer whose refcount/length is only valid for a single consume (a borrow of the view's backing
; that the first Bytes.at frees or advances), so the second read sees zeros / walks off the end.
; A concat source (owned) and an ASCII slice are fine; only the multibyte VIEW leaks.
;
; Severity HIGH: silent wrong bytes (t7/t8/t12) on wasm — the DEFAULT backend — and an OOB trap in
; the inline form. to-bytes of a substring read more than once is an ordinary parsing shape.

(case "to-bytes of a sliced multibyte string is read twice and both reads see the right bytes"
  (doc    "The to-bytes buffer of a String.slice VIEW must be independently readable N times. `tail`
           = slice(concat 'ab' 'cdé', 3, 5) = 'dé' (bytes [100, 0xC3, 0xA9]); `b = to-bytes tail`;
           `b[0] + b[1]` = 100 + 195 = 295. Wasm returns 100 (b[1] reads 0 — the buffer is consumed
           by the first read); rust/rust-async compute 295. Graded against the CORRECT (rust) value;
           red on wasm until fixed.")
  (input  (do
            (def (main (: k Int64))
              (let ((s (String.concat "ab" "cdé")))
                (match (String.slice s 3 5)
                  ((Some tail)
                    (let ((b (String.to-bytes tail)))
                      (+ (Int64.of (Option.expect (Bytes.at b 0) "b0"))
                         (Int64.of (Option.expect (Bytes.at b 1) "b1")))))
                  ((None u) -1))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 295 Int64)))
