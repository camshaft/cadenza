; adv-50 (breaker tick 1020) — INVALID ARTIFACT on BOTH backends: a CAPTURING closure that (a) is
; stored into a CHAMP container (Map value — even with the insert result DISCARDED) and (b) is ALSO
; called directly, emits a broken artifact:
;   wasm:      "invalid component: failed to compile: wasm[0]::function[N]" (invalid module, trap at load)
;   rust:      error[E0425]: cannot find value `__cap0` in this scope (artifact does not build)
;   rust-async: same E0425
; NOT a decline — the compiler ACCEPTS the program and emits garbage. Reject-don't-miscompile violated
; at the artifact level (no wrong VALUE escapes since nothing runs, but the emit is broken).
;
; BOUNDARY (shrunk tick 1020, trunk 1398dce15):
;   - capture REQUIRED: non-capturing closure in map + direct call → PASS (s8)
;   - scalar capture suffices (k Int64); heap capture (list) also fails (s7/s9)
;   - CHAMP entry REQUIRED: tuple container + direct call → PASS (s6); no container → (long-pinned) PASS
;   - the DIRECT call is the trigger: lookup-only call → PASS (s4, matches the pinned dispatch-table
;     idiom 09-functions:554); insert-only without direct call → PASS (s11)
;   - overwrite/lookup NOT required: insert result entirely DISCARDED still fails (this repro)
; Suspected: the closure-env materialization for the CHAMP value cell (boxed fn handle) conflicts with
; the direct-call inlining/env-slot path when the SAME closure binding is used both ways — the rust
; E0425 __cap0 suggests the captured-var slot is renamed/erased on one path while the other still
; references it. Both backends broken → likely shared lower/core, not per-backend emit.
;
; Expected (hand-derived): k=100 captured; f1 v = k+v; main d=5 → insert discarded → f1 5 = 105.

(case "a capturing closure stored into a map AND called directly compiles to a valid artifact"
  (input  (do
            (def (main (: d Int64))
              (let ((k 100))
                (let ((f1 (fn ((: v Int64)) (+ k v))))
                  (do (Map.insert Map.empty 1 f1)
                      (f1 d)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 105 Int64)))
