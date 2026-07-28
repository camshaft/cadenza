; FINDING (breaker, 2026-07-21): an out-of-range literal in a RECORD FIELD or USER-SUM payload
; escapes the width fit-check entirely — check accepts, then wasm SILENTLY TRUNCATES the value
; (999 as Int8 → -25) while rust fails to build (E0308) — a backend-divergent miscompile class.
;
; The compound-payload width descent covers the BUILT-IN sum (Option: (: (Some 999) (Option
; Int8)) rejects; float twin just pinned) but has no arm for:
;   - RECORD fields:      (: (record (x 999)) (Record (: x Int8)))      → accepted, wasm x = -25
;   - single-payload SUM: (type W (W Int8))  (: (W 999) W)              → accepted
;   - multi-payload SUM:  (type P (P Int8 Int8)) (: (P 999 5) P)        → accepted
; Same for the FLOAT family: (: (record (x 1.0e300)) (Record (: x Float32))) and
; (: (W 1.0e300) W) / (: (P 1.0e300 0.5) P) with Float32 payloads are all accepted at check
; (the const whole-value render even prints the full f64 expansion in a Float32 field).
;
; Runtime witness (wasm): (. (: (record (x 999)) (Record (: x Int8))) x) RUNS → -25
; (999 & 0xFF = 231 = -25 as i8 — silent two's-complement truncation, verified deterministic).
; The rust backend rejects the same program at rustc-build time with E0308 mismatched types —
; so the backends DIVERGE on accepted-program behavior: one truncates, one doesn't build.
;
; Both int and float families, wasm + rust. Expected: CDZ0302 at check, exactly as the Option
; payload and the bare-annotation forms — the descent needs record-field + user-sum-payload arms.

(case "REPRO an Int8-overflowing literal in a record field is rejected"
  (input  (: (record (x 999)) (Record (: x Int8))))
  (error  CDZ0302))

(case "REPRO an Int8-overflowing literal in a single-payload user sum is rejected"
  (input  (do
            (type W (W Int8))
            (def (main) (: (W 999) W))
            (export main)))
  (error  CDZ0302))

(case "REPRO an Int8-overflowing literal in a multi-payload user sum is rejected"
  (input  (do
            (type P (P Int8 Int8))
            (def (main) (: (P 999 5) P))
            (export main)))
  (error  CDZ0302))

(case "REPRO a Float32-overflowing literal in a record field is rejected"
  (input  (: (record (x 1.0e300)) (Record (: x Float32))))
  (error  CDZ0302))

(case "WITNESS today the wasm record field silently truncates (999 -> -25); MUST become CDZ0302"
  (input  (do
            (def (main)
              (. (: (record (x 999)) (Record (: x Int8))) x))
            (export main)))
  (error  CDZ0302))
