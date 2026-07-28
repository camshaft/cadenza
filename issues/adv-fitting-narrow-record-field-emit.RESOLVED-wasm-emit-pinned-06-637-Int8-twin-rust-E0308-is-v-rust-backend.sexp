; FINDING (breaker, 2026-07-21, post bb43b5d0a): a FITTING narrow-width literal in a RECORD
; FIELD is broken at EMIT — the check side (bb43b5d0a part 1) now correctly rejects the
; OVERFLOWING record-field literal, but the valid program with a fitting value fails:
;
;   Int8 field, rust:    (def (get (: r (Record (: x Int8)))) (. r x)) (get (record (x 100)))
;                        → rustc E0308 mismatched types            [wasm computes 100]
;   Float32 field, rust: same shape at (: x Float32), (record (x 1.5))
;                        → rustc E0308                              [wasm: INVALID MODULE
;                          (cdz-run: failed to compile wasm[0]::function[6])]
;   Int64 field:         computes on both (control)
;   Int8 TUPLE element:  (. (: (tuple 100 5) (Tuple Int8 Int8)) 0) computes on rust (control —
;                        the tuple path grounds correctly; only the RECORD path is broken)
;   annotation position, let-bound, or helper-param — all Int8-record shapes fail rust alike.
;
; Likely root: the record-field literal never grounds to the declared narrow field type on the
; emit path (stays i64/f64), so the rust struct literal has an i64 in an i8 field (E0308) and
; the wasm Float32 case pushes f64 against an f32 slot (invalid module) — the RECORD sibling of
; the fitting-Float32-branch bug ea2be74b5 just fixed for if-branches, and the fitting-side
; twin of the width-check audit's record arm. The same audit slice that adds user-sum payload
; rejects should ground the FITTING record-field literal at its declared type.
;
; wasm-side severity: Float32-in-record is UNUSABLE (invalid module for every valid program).
; rust-side severity: every narrow-int/float record field with a literal init fails to build.

(case "REPRO a fitting Int8 record field computes (rust E0308 today)"
  (input  (do
            (def (get (: r (Record (: x Int8)))) (. r x))
            (def (main) (get (record (x 100))))
            (export main)))
  (call   main) (output (: 100 Int8)))

(case "REPRO a fitting Float32 record field computes (rust E0308 + wasm invalid module today)"
  (input  (do
            (def (get (: r (Record (: x Float32)))) (. r x))
            (def (main) (get (record (x 1.5))))
            (export main)))
  (call   main) (output (: 1.5 Float32)))

(case "CONTROL the Int64 field and Int8 tuple element compute"
  (input  (do
            (def (get (: r (Record (: x Int64)))) (. r x))
            (def (main)
              (+ (get (record (x 100)))
                 (Int64.of (. (: (tuple 100 5) (Tuple Int8 Int8)) 0))))
            (export main)))
  (call   main) (output (: 200 Int64)))
