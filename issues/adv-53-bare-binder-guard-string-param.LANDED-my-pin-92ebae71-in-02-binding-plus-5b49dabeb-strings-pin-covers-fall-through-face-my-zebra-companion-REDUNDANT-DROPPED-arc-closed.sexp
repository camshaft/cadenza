; adv-53 (breaker tick 1099) — OVER-REJECT (finding-#46 residual class): a bare-binder GUARD over a
; STRING (heap-typed) PARAM scrutinee in a non-entry helper rejects CDZ0101 'unbound name' on ALL
; THREE targets, while the Int64 twin — the exact #46 control shape — compiles and runs.
;
; SHRINK (tick 1099, /tmp/breaker-shrink2 g1-g5):
;   g4 (guard w (> w 10)) over an Int64 PARAM in a helper     -> RUNS (the #46 fixed control)
;   g2 same STRING guard, scrutinee directly in MAIN          -> RUNS
;   g5 (guard (Some t) (< t "m")) — Some-PAYLOAD String guard -> RUNS
;   g1 (guard t (< t "m")) over a String PARAM in a helper    -> CDZ0101  <- THIS
;   g3 same but guard reads byte-len (scalar compare)         -> CDZ0101 (so it's the BINDER, not <)
; Trigger = bare-binder guard x HEAP-typed param scrutinee x non-entry frame. The #46 fix
; (0f79b082f) repaired the COMPUTED-scrutinee orphan for scalars; the heap-param face still orphans
; the guard binder (t never binds, the guard body's read leaks to scoping -> CDZ0101).
;
; Severity moderate: over-reject (no wrong value), but the shape is the natural string-classifier
; helper (band/bucket by name) and the CDZ0101 message is misleading.

(case "a bare-binder guard over a String param scrutinee in a helper binds and evaluates"
  (doc    "The heap-param face of the guarded-scalar desugar (finding #46 fixed the computed-scalar
           face; its raw-param control was Int64-only): `(match s ((guard t (< t \"m\")) 1) (_ 3))`
           with `s : String` a helper param must bind t and run the guard — \"apple\" < \"m\" → 1.
           Today all 3 targets reject CDZ0101 (the guard binder orphans; graded against the SPEC, red
           until fixed).")
  (input  (do
            (def (band (: s String))
              (match s ((guard t (< t "m")) 1) (_ 3)))
            (def (main (: k Int64)) (band "apple"))
            (export main)))
  (call   main (: 0 Int64)) (output (: 1 Int64)))
