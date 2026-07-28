; ===== PM triage (corpus-bugfix, trunk 9b3a0f47a) — VERIFIED reproducing + ROUTED =====
; VERIFIED: (def (main (: n Int64)) (: (if (< n 5) 1.5 0.25) Float32)) => 'cdz: invalid component:
; failed to compile: wasm[0]::function[0]'. Branch literals emit f64.const while the outer Float32
; annotation makes the if-block type f32 → invalid module. PASSES with per-branch annotations / Float64 /
; the Int8 analogue, so the OUTER float-width annotation must push width DOWN into each bare branch literal
; at EMIT time (check side already fixed by v-inference ffa733b67 — this is the check-vs-emit continuation).
; ROUTED: wasm-emit gap -> v-inference (their literal-width-grounding lane). rust MATCH-form E0308 sibling
; -> v-rust-backend (match-arm literal renders f64 vs f32 result). corpus-bugfix pins a fitting-Float32-branch
; case across all 3 backends once the emit is fixed.
; FINDING (breaker, 2026-07-21): a FITTING Float32 literal in a RUNTIME conditional under a
; Float32 annotation emits an INVALID wasm module (cdz-run: invalid component: failed to
; compile: wasm[0]::function[0]). The check side is fine — this is the emit path.
;
; Context: ffa733b67 (v-inference, landed 8dd2b2ac8) closed the check-vs-emit gap for the
; OVERFLOWING branch literal ((: (if c 1.0e300 0.0) Float32) now rejects CDZ0302 at check).
; But the VALID program — a fitting literal in the same position — still emits a broken
; module. Likely the branch literal is emitted at f64 width (f64.const) while the annotated
; conditional's result type is f32, so the wasm block type disagrees with the branch value.
;
; Scope (all verified at gate default O1, and opt-sweep O0..O3 = same outcome at every level):
;   FAILS  (: (if c 1.5 0.25) Float32)                      — annotated runtime if
;   FAILS  (: (match n (0 0.5) (_ 1.5)) Float32)            — annotated runtime match
;   FAILS  (let ((x (: (if c 1.5 0.25) Float32))) x)        — let-bound
;   FAILS  (f (if c 1.5 0.25)) with (def (f (: x Float32))) — param-annotation context
;   FAILS  (: (if c a 0.25) Float32) with (: a Float32)     — MIXED param + literal branch
;   FAILS  (+ a (: (if c 1.5 0.25) Float32))                — feeding runtime f32 arith
;   FAILS  (: (if c (if d 1.5 2.5) 0.25) Float32)           — nested conditionals
;   OK     (if c (: 1.5 Float32) (: 0.25 Float32))          — PER-BRANCH annotations
;   OK     (if c a b) over two Float32 params               — no literal branches
;   OK     (: (if true 1.5 0.25) Float32)                   — const-foldable conditional
;   OK     (: (if c 1.5 0.25) Float64)                      — Float64 (wide) annotation
;   OK     (: (if c 5 7) Int8)                              — narrow-INT analogue works
;   OK     (: 1.5 Float32)                                  — direct literal, no conditional
;
; rust backend: the if-forms PASS on rust; the MATCH form fails rustc E0308 (mismatched
; types) on BOTH rust and rust-async — filed as the sibling finding in the same issue.
;
; Repro cases (expected values are the obvious ones; today they trap invalid-module):

(case "REPRO fitting Float32 literal in a runtime if branch computes"
  (input  (do
            (def (main (: c Bool))
              (: (if c 1.5 0.25) Float32))
            (export main)))
  (call   main (: true Bool))  (output (: 1.5 Float32))
  (call   main (: false Bool)) (output (: 0.25 Float32)))

(case "REPRO fitting Float32 literal in a runtime match arm computes"
  (input  (do
            (def (main (: n Int64))
              (: (match n (0 0.5) (_ 1.5)) Float32))
            (export main)))
  (call   main (: 0 Int64)) (output (: 0.5 Float32))
  (call   main (: 3 Int64)) (output (: 1.5 Float32)))

(case "REPRO mixed param-and-literal Float32 branches compute"
  (input  (do
            (def (main (: c Bool) (: a Float32))
              (: (if c a 0.25) Float32))
            (export main)))
  (call   main (: true Bool) (: 1.5 Float32))  (output (: 1.5 Float32))
  (call   main (: false Bool) (: 1.5 Float32)) (output (: 0.25 Float32)))
