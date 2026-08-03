; adv-63 (breaker, 2026-08-03, MED wrong-REJECT — valid programs rejected, position-dependent):
; a GENERIC sum whose declared name coincides with its sole variant's name — (type Box (Box a)) —
; misresolves the BARE constructor `(Box 7)` as the TYPE constructor when written inside a def
; that has AT LEAST ONE parameter, rejecting CDZ0203 "`Box` is a type constructor — its type
; argument must be a type, but a value appears here". The SAME expression in a NULLARY def
; compiles and runs.
;
; isolation matrix (all on trunk 3a6bff4b9):
;   (def (main) (match (Box 7) ...))                      -> 7          OK (nullary)
;   (def (main (: k Int64)) (match (Box 7) ...))          -> CDZ0203    WRONG (param'd, const payload)
;   (def (main (: _k Int64)) ...)                         -> CDZ0203    WRONG (unused param too)
;   (def (main (: k Int64)) (match (Box k) ...))          -> CDZ0203    WRONG (runtime payload)
;   (def (main (: k Int64)) (match (Box.Box k) ...))      -> 6          OK (QUALIFIED ctor)
;   (type Wrap (Mk a)) + (def (main (: k Int64)) (Mk k))  -> 6          OK (distinct variant name)
;   nullary helper w/ bare (Box 7), called from param'd main -> 12      OK (the resolution is per-DEF)
;   corpus pin 07-type-system:1244 (Holder-wrapped)       -> nullary main, so this face was invisible
;
; expected: a VALUE position (Box 7) is the variant constructor regardless of the enclosing def's
; arity — the name-coincidence disambiguation (value-position => variant) must not depend on
; whether the def has parameters. workaround exists (qualified Box.Box) but the bare spelling is
; the corpus-blessed one (the Holder pin uses bare (Box 7) in a nullary main).
(case "adv-63 a name-coincident generic ctor in VALUE position resolves in a PARAMETERIZED def too"
  (input  (do
            (type Box (Box a))
            (def (main (: k Int64)) (match (Box k) ((Box v) (+ v 1))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 6 Int64)))

; --- CORPUS-BUGFIX RE-TRIAGE (2026-08-03, fresh rebuild, base 1f7d8e7ab == trunk resolve) ---
; The FILED HEADLINE no longer reproduces: (def (main (: k Int64)) (match (Box k) ((Box v) v))) now
; PASSES (value 5). The in-place-match-in-a-param'd-def face appears already fixed.
; The LIVE bug is a MISCOMPILE (not a wrong-reject) on a different shape — a param'd def that RETURNS
; a bare name-coincident generic ctor VALUE emits INVALID WASM, while rust is correct (DIFFERENTIAL):
;   (def (main (: k Int64)) (Box k))               -> wasm: invalid component / failed to compile; rust: 5
;   (def (main (: k Int64)) (let ((b (Box k))) b)) -> same wasm-invalid
;   (def (main) (Box 5))  [nullary]                -> value (: 5 Box), fine
; Trigger = name-coincident ctor value ESCAPING a PARAMETERIZED def. Routed to v-inference as a
; MISCOMPILE. Pin on their fix (the Box value's render shape governs the expected output).
(case "adv-63 a param'd def RETURNING a name-coincident generic ctor value emits valid wasm (differential; wasm invalid, rust ok)"
  (input  (do
            (type Box (Box a))
            (def (main (: k Int64)) (match (Box k) ((Box v) (+ v 1))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 6 Int64)))

; --- CONVERGED TRIGGER (breaker + corpus-bugfix, fresh builds, 2026-08-03) ---
; The real root is INLINE-INTO-CALLER re-resolution — NOT def arity, NOT specifically 'returning'.
; A name-coincident generic ctor rejects CDZ0203 only when the ctor-using def is INLINED INTO A
; CALLER (the inline pass re-resolves the ctor head in the caller's context and hits the TYPE binding
; first). Direct-export any-arity is fine; qualified Box.Box is fine; a distinct variant name is fine.
; My earlier 'param'd def RETURNING (Box k) emits invalid wasm' face is the SAME root one emit-stage
; later. ONE bug for v-inference: keep value-position => variant across the inline re-resolution.
; These two are the gate-runnable todo faces (flip to pass on fix):
(case "adv-63 a name-coincident generic ctor in a def CALLED from another def resolves (inline, runtime arg)"
  (input  (do
            (type Box (Box a))
            (def (inner (: k Int64)) (match (Box k) ((Box v) v)))
            (def (main (: n Int64)) (inner n))
            (export main)))
  (call   main (: 5 Int64)) (output (: 5 Int64)))
(case "adv-63 a name-coincident generic ctor in a def called from another def resolves (inline, const arg)"
  (input  (do
            (type Box (Box a))
            (def (inner (: k Int64)) (match (Box k) ((Box v) v)))
            (def (main (: n Int64)) (inner 7))
            (export main)))
  (call   main (: 5 Int64)) (output (: 7 Int64)))
