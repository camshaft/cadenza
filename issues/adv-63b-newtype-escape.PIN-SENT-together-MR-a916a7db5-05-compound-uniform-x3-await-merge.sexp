; adv-63b (split from adv-63 by v-inference triage, 2026-08-03, MED-HIGH wasm soundness):
; a SINGLE-VARIANT (erased) NEWTYPE value RETURNED from a PARAMETERIZED def emits INVALID WASM
; ('invalid component: failed to compile: wasm[1]::function[9]') while rust computes correctly.
; DIFFERENTIAL. This is NOT name-coincidence and NOT a resolve bug (v-inference: it checks clean +
; resolves fine as a direct user node; the adv-63 inline-face was a separate resolve bug, fixed in
; d322608b4). Root: the erased-newtype RETURN ABI on the wasm backend — the newtype value escaping a
; param'd def is mis-emitted.
;
; isolation (corpus-bugfix, trunk 9189e53d3):
;   (type Box (Mk a)) (def (main (: k Int64)) (Mk k))     -> wasm INVALID; rust value 5   [generic]
;   (type W (Mk Int64)) (def (main (: k Int64)) (Mk k))   -> wasm INVALID; (rust n/c)      [NON-generic too]
;   (def (main) (Mk 5))  [nullary]                        -> fine (renders (: 5 Box))
;   (match (Mk k) ((Mk v) v)) direct-export               -> fine (value 5)
;   (def (main (: k Int64)) (Some k)) [multi-variant, no erase] -> fine
; So the trigger is: erased single-variant newtype VALUE + ESCAPE from a PARAMETERIZED def, wasm only.
; Route: v-wasm-opt (owns the wasm value-op emit producing the invalid module) / coordinate with
; v-rust-backend if the erased-newtype return ABI seam is shared. PIN-ON-FIX (differential; wasm
; should emit valid module returning the newtype value, matching rust).
(case "adv-63b a single-variant newtype value returned from a parameterized def emits valid wasm"
  (input  (do
            (type Box (Mk a))
            (def (main (: k Int64)) (match (Mk k) ((Mk v) v)))
            (export main)))
  (call   main (: 5 Int64)) (output (: 5 Int64)))
