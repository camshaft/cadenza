; FINDING (breaker, 2026-07-27): `String.to-bytes` of a RUNTIME rope (String.concat-built), with the
; resulting Bytes value used across FOUR OR MORE branch arms, emits an INVALID WASM MODULE:
;   wasm-tools validate: "func N failed to validate: type mismatch: expected i32, found i64"
; Compile succeeds (module written); every RUN fails "invalid component: failed to compile:
; wasm[0]::function[N]". Both wasm targets affected the same way; discovered while probing
; to-bytes over a seam-crossing slice view, but the SLICE is irrelevant.
;
; MATRIX (def bs (String.to-bytes s)) where s = (String.concat ...) runtime rope:
;   FAIL  4 uses: len + at0 + at2 + from-bytes            (m22, multibyte rope)
;   FAIL  4 uses: len + at0 + at2 + at3                   (m20 rope+slice; m29 ASCII fold-defeated)
;   FAIL  4 uses: len + at0 + at2 + from-bytes(_t ignored) (m17 — string-eq not needed)
;   ok    3 uses: len + at0 + from-bytes                  (m27)
;   ok    3 uses: len + at0 + at2                         (m15, even WITH a slice view in the chain)
;   ok    2 uses: len + at0                               (m28)
;   ok    4 uses but CONST-FOLDABLE rope (all-literal, no runtime operand) — folds away (m24/m26)
;   ok    4 uses of to-bytes of a NON-rope runtime string (entry-arg identity fn, m23 folds; and
;         flat-literal slice m21 folds — no non-folding non-rope repro found, rope may be required)
;
; The i64-found-where-i32-expected smells like a Bytes handle (i64?) vs byte/len (i32) width
; confusion on a SHARED local when the same to-bytes result is spilled/reused across enough arms —
; a use-count-triggered path (4th use = re-materialization?). Emit-side, not check-side: the
; program typechecks and the module is written.
;
; REPRO (deterministic, fails at run on wasm; rust targets likely fine — untested pending routing):
(case "String.to-bytes of a runtime rope reused across four branch arms compiles to a VALID module"
  (input (do
        (def (main (: mode Int64))
          (do
            (def s (String.concat "ab" (if (< mode 100) "cd" "zz")))
            (def bs (String.to-bytes s))
            (if (= mode 1) (Bytes.len bs)
                (if (= mode 2) (match (Bytes.at bs 0) ((Some b) b) ((None _u) -1))
                    (if (= mode 3) (match (Bytes.at bs 2) ((Some b) b) ((None _u) -1))
                        (match (Bytes.at bs 3) ((Some b) b) ((None _u) -1)))))))
        (export main)))
  (call main (: 1 Int64)) (output (: 4 Int64))
  (call main (: 2 Int64)) (output (: 97 Int64))
  (call main (: 3 Int64)) (output (: 99 Int64))
  (call main (: 4 Int64)) (output (: 100 Int64)))
