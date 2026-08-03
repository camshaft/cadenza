; FINDING #47 (breaker, 2026-08-01): the cadenza-ml front-end does NOT enforce parameter-list
; linearity — two same-named fn params are accepted and the second silently SHADOWS the first.
; rcdzc rejects CDZ0102 (all 3 targets); ML runs to a wrong-looking value. Third genuine gap in
; the validation-walk class pr-sync bisected out of drain AT (reject 000000019321): export-unbound
; + unbound-payload-type (both CDZ0101, ml=value 42) + THIS (CDZ0102, ml=value 2).
; Per the enforcing-gate ruling these are FIX-in-compiler-ml, not KNOWN_ML_DIFFS entries.
; The corpus pin is HELD at breaker (/tmp/breaker-banked-dup-param.sexp) until the ML fix lands.
;
; Repro: ./target/release-debug/cdz run-ml <this program>  →  "value 2" (expected: reject CDZ0102)

(case "a function with two SAME-NAMED parameters is a non-linear binding rejection"
  (input  (do (def (f (: x Int64) (: x Int64)) x) (def (main) (f 1 2)) (export main)))
  (error  CDZ0102))
