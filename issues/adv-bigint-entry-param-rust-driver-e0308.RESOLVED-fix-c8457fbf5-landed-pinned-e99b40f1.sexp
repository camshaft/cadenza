; BREAKER FINDING — a BigInt ENTRY PARAMETER on the rust targets emits an artifact that does NOT BUILD
; (rustc E0308 mismatched types), while the check side ACCEPTS the program. The driver's argument
; marshaling does not match the emitted fn signature for a BigInt param — the exact class of the FIXED
; String-entry-arg E0308 (13-strings:2575 family: driver now passes "abc".to_string(); BigInt needs the
; equivalent owned-BigInt construction). Rational entry params WORK on rust (control below). wasm
; DECLINES the BigInt entry arg (sound todo, same as String); rust/rust-async FAIL with no-build.
;
; Grades today: wasm todo(decline) / rust FAIL(E0308) / rust-async FAIL(E0308).
; Expected after fix: rust computes 5000000 / -3000000 (then pin per-target like the String family).

(case "a BigInt entry parameter is marshaled by the rust driver"
  (input (do
           (def (main (: a BigInt)) (* a (BigInt.of 1000000)))
           (export main)))
  (call main (: 5 BigInt))
  (output (: 5000000 BigInt))
  (call main (: -3 BigInt))
  (output (: -3000000 BigInt)))

; CONTROL — the Rational twin PASSES on rust/rust-async today (1/2), declines on wasm. Only BigInt's
; driver marshal is broken.
(case "CONTROL a Rational entry parameter computes on rust"
  (input (do
           (def (main (: a Rational)) (* a (Rational.of 2 3)))
           (export main)))
  (call main (: 3/4 Rational))
  (output (: 1/2 Rational)))

; Also-affected face: the annotated-big-literal body form hits the same E0308.
(case "a BigInt entry parameter plus an annotated big literal"
  (input (do
           (def (main (: a BigInt)) (+ a (: 100000000000000000000 BigInt)))
           (export main)))
  (call main (: 1 BigInt))
  (output (: 100000000000000000001 BigInt)))

; ADDENDUM (same tick): the RATIONAL driver marshal has its own narrower face — a BARE-INT argument
; literal ((call main (: 0 Rational))) also E0308-no-builds; spelled 0/1 it passes. So: n/d Rational
; args marshal fine, bare-int Rational args and ALL BigInt args do not.
