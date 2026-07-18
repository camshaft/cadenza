; BREAKER FINDING 2026-07-18 (trunk aaf51027c, post handle-result-applyability fix 331283703) —
; OVER-CONSERVATIVE ESCAPE REJECT: a closure that merely CAPTURES a value derived from a perform
; (the perform itself evaluated INSIDE the handle body, before the closure exists) is rejected
; CDZ0401 "no home" when the closure ESCAPES the handle as its result:
;
;   (handle St k ((get (u) s (resume s s)))
;     (let ((v (St.get)))                 ; perform runs UNDER the handler; v is a plain Int64
;       (fn ((: x Int64)) (+ x v))))      ; closure captures the VALUE v — performs nothing
;   applied outside: ((handle …) 10)     -> CDZ0401 at the St.get (6:17)
;   helper-built twin (mkfn (St.get))    -> CDZ0401 too (the perform is a plain evaluated ARG)
;
; CONTROLS (all verified this base):
;   - same let+perform, VALUE body (+ v 1)                    -> runs (8) ✓
;   - ((mkfn (St.get)) 10) applied INSIDE the handle          -> runs (17) ✓
;   - closure body PERFORMING (fn … (St.get)) escaping        -> CDZ0401 ✓ CORRECT (the pinned
;     escape rule: an escaping closure may not carry a live perform)
;   - plain (fn (x) (+ x 1)) escaping the handle, applied out -> runs (11) ✓ (the 331283703 fix)
;
; So the escape analysis conflates "a perform occurs lexically in the handle body that produces an
; escaping closure" with "the escaping closure itself performs". The captured `v` is an ordinary
; Int64 evaluated to a value while the handler was live; the escaping closure is pure. The legal
; discharge-then-capture idiom (compute under the handler, close over the RESULT) is rejected with
; the same diagnostic as the genuinely-unsound escape, and the diagnostic points at the St.get —
; which IS homed — rather than at any actual escape.
;
; SEVERITY: reject-gap (no miscompile), but the idiom is the natural "configure a callback from
; handled state" pattern, and the false CDZ0401 is indistinguishable from the real escape error.
;
; Expected: k=7 -> v=7 captured, closure escapes, applied to 10 -> 17.
(case "a closure capturing a value computed under a handler may escape the handle"
  (doc    "The perform `(St.get)` runs INSIDE the handle body (let init — the handler is live), and
           the escaping closure captures only the resulting Int64 VALUE: discharge-then-capture. The
           closure performs nothing, so the escape is sound — `((handle … (let ((v (St.get))) (fn (x)
           (+ x v)))) 10)` with k=7 is 17, exactly as applying it INSIDE the handle already computes.
           Currently rejected CDZ0401 'no home' AT THE St.get (which is homed), conflating a
           lexically-inner perform with an escaping one; the genuinely-unsound twin (the closure BODY
           performing) is correctly rejected and must stay so.")
  (input  (do
            (effect St (op get (-> Unit Int64)))
            (def (main (: k Int64))
              ((handle St k
                 ((get (u) s (resume s s)))
                 (let ((v (St.get)))
                   (fn ((: x Int64)) (+ x v))))
               10))
            (export main)))
  (call   main (: 7 Int64))
  (output (: 17 Int64)))

; ---
; ROUTED to v-effects (corpus-bugfix 2026-07-18, VERIFIED trunk aaf51027c rebuilt): escape-analysis
; OVER-REJECT, newly reachable after v-inference's 331283703 handle-result-applyability fix. (handle St 8
; (arm) (let ((v (St.get))) (fn (x) (+ x v)))) applied outside -> CDZ0401 at the St.get, but the perform
; runs INSIDE the handle, the closure captures only Int64 v (pure), and applying it INSIDE runs (17). Escape
; analysis wrongly attributes a lexically-inner perform to the escaping closure instead of checking the
; CLOSURE BODY. The unsound twin (closure BODY performs) correctly rejects + MUST stay (E5 captured-k).
; Reject-gap, no miscompile; diagnostic misleads (points at homed St.get). Same escape machinery as the
; nested-collection escape routing (landed). Not spawning (v-effects territory). Promote when fixed.
