; FINDING (breaker, 2026-07-24): a `do`-def local is CDZ0101 "unbound name" when referenced
; from the ARGUMENT of a perform inside a handle body — a FALSE REJECT. The semantically
; identical `let`-bound form compiles and computes CORRECTLY on all 3 targets (wasm/rust/
; rust-async), so this is a frontend/desugar scoping gap in `do` bodies under `handle`,
; specific to the perform-argument position.
;
; MATRIX (all minimal, scalar-only — no heap involvement needed):
;   ✗ (do (def v (+ u 2)) (Bail.bail v))            — abortive, do-def in perform arg    → CDZ0101 unbound v
;   ✗ (do (def v (+ u 2)) (+ (Ask.ask v) 1))        — resuming, same                     → CDZ0101
;   ✗ (do (def v (+ u 2)) (+ (Ask.ask v) v))        — do-def in arg AND after            → CDZ0101
;   ✗ (do (def rope (rep …)) (Bail.bail (String.byte-len rope)))  — heap do-def in arg   → CDZ0101
;   ✗ (do (def v (+ u 2)) (+ (poke v) 1)) where (def (poke (: v Int64)) (Ask.ask v))
;         — do-def passed to a HELPER that performs                                       → CDZ0101
;   ✓ (let ((v (+ u 2))) (+ (Ask.ask v) v))          — LET-bound, same shape             → computes 21, all 3 targets
;   ✓ (do (def v (+ u 2)) (+ (Ask.ask 3) (twice v))) — do-def in a NON-perform arg beside a perform → OK
;   ✓ (+ (Ask.ask u) 1)                              — PARAM in perform arg              → OK
;   ✓ (do (def rope (rep …)) (+ (Bail.bail 7) (byte-len rope))) — do-def AFTER a const-arg perform → OK (ctl5)
;
; So: `do`-def + perform coexist fine UNLESS the def flows into the perform's argument
; (directly or via a call chain that performs). Likely the perform-argument lowering
; captures/rescopes the surrounding do-bindings differently from ordinary call arguments.
;
; IMPACT: any program computing a value in a do-block and performing with it — the natural
; effectful-code shape (compute, then ask/log/bail with the result) — falsely rejects.
; WORKAROUND exists (let-bind instead), which is how the corpus never hit it: no landed
; case flows a do-def into a perform arg.
;
; Minimal repro (this file's case): expect 21 ((ask 7)→14 resumed, +7); actual = CDZ0101
; unbound name `v` at check on all 3 targets.

(case "a do-def value flows into a perform argument (FALSE-REJECT repro: CDZ0101 unbound)"
  (input (do
        (effect Ask (op ask (-> Int64 Int64)))
        (def (run (: u Int64))
          (handle Ask 0
            ((ask (n) s (resume (* n 2) s)))
            (do
              (def v (+ u 2))
              (+ (Ask.ask v) v))))
        (def (main) (run 5))
        (export main)))
  (output (: 21 Int64)))
; NARROWING (breaker #21 boundary): do-def into RESUME-arg in an ARM is FINE; only PERFORM-arg in BODY rescopes. Reference path for v-effects to diff.
