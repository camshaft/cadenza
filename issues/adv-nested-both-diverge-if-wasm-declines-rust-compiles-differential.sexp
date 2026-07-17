; BREAKER FINDING 2026-07-17 — a DIFFERENTIAL (wasm declines, rust+rust-async COMPILE+RUN):
; a both-diverge if used as a SUBEXPRESSION of an otherwise-well-typed value if.
;
;   (def (main (: b Bool) (: c Bool)) (if b 1 (if c (trap "x") (trap "y"))))
;     outer then = 1 (Int64); outer else = (if c (trap)(trap)) = Never; outer if unifies to Int64.
;     call (b=true, c=false) -> 1.
;   cdz check -> PASSES rc=0 (infer types it correctly; the outer if is Int64, the inner is Never).
;
; BACKEND SPLIT:
;   wasm       -> DECLINES "if result type has no machine representation" (todo)
;   rust       -> COMPILES + runs -> 1 (pass)
;   rust-async -> COMPILES + runs -> 1 (pass)
;
; So the RUST backend already handles a both-diverge if as a SUBEXPRESSION (the outer if has a concrete
; Int64 type; the Never inner is threaded fine). The WASM backend declines the WHOLE program because its
; if-emit (select.rs ~:4581) hits the inner both-diverge if, computes block_ty from its Never result,
; valtype_of=None -> bails. This is WIDER than the top-level both-diverge case (which is a UNIFORM decline
; on both backends, not a differential): a both-diverge if/match ANYWHERE in the tree — even nested inside
; a well-typed value expression — reds only wasm. A common idiom this blocks: (if cond value (if sub (trap)
; (trap))) — an "impossible else, exhaustively trap" fallback.
;
; FIX (same as the top-level case, v-inference ruled option (a) + routed to v-wasm-opt): the wasm if/match
; emit should emit BlockType::Empty when the (sub)expression's result is Never. The rust backend shows the
; target behavior. Precedent: select.rs:2845/2935 (direct Never-body fn already emits 0-result). Once fixed,
; wasm matches rust -> this case COMPILES + runs to 1 on all 3 backends.
;
; Expected under the fix: value 1 on all 3 backends (the DIFFERENTIAL closes; wasm joins rust+async).
(case "a both-diverge if nested in a value if compiles (the outer concrete arm gives the machine type)"
  (doc "(if b 1 (if c (trap) (trap))) — outer if is Int64 (then=1), inner both-diverge if is Never and
        unifies into the else. Currently a DIFFERENTIAL: rust+rust-async compile+run to 1, wasm DECLINES
        'if result type has no machine representation'. b=true -> 1. Expected under the ruled (a) wasm
        if-emit fix (BlockType::Empty for a Never result): 1 on all 3 backends, closing the differential.")
  (input (do (def (main (: b Bool) (: c Bool)) (if b 1 (if c (trap "x") (trap "y")))) (export main)))
  (call  main (: true Bool) (: false Bool))
  (output (: 1 Int64)))
