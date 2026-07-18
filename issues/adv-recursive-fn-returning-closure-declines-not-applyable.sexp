; BREAKER FINDING 2026-07-18 (trunk 3c4a6341a era, base a26f90b59) — DECLINE-GAP (reject-not-miscompile,
; check rc=0, both backends): a RECURSIVE def whose RESULT is a closure cannot be applied — the call
; site declines `value is not applyable` (terse, no code, no hint).
;
;   (def (selfp (: n Int64))
;     (if (= n 0) (fn ((: x Int64)) (+ x 100)) (selfp (- n 1))))
;   (def (main (: n Int64)) ((selfp n) 5))          -> "value is not applyable"  [wasm AND rust]
;   let-bound twin ((let ((g (selfp n))) (g 5)))    -> same decline
;   MUTUAL twin (evenf/oddf each returning a fn)    -> same decline
;   cdz check on all: rc=0 (well-typed).
;
; CONTROLS (all work):
;   - NON-recursive closure-returner ((pick n) 5)   -> runs (105/205)
;   - closures INTO recursion (the pinned 09-functions:927 runtime-selected-closure-through-
;     recursive-HOF pair) -> run
; So the asymmetry: a closure value flows INTO a recursive def fine, but cannot flow OUT of one —
; the recursion's result type evidently never resolves to an applyable Ty::Fn at the call site
; (adjacent to, but distinct from, the known currying/nested-apply-head "not applyable" faces in
; rcdzc-runtime-closures-design and the closure-ARG mono-ceiling tie; this is the closure-RESULT
; column of the recursion table).
;
; SEVERITY: decline-gap with a poor diagnostic — the factory-selected-by-recursion idiom
; (`resolve-handler(depth)` returning a handler fn) is natural code a self-hosted pass will write.
; Also the message has no error CODE and no hint (compare the excellent no-base-case diagnostic).
;
; Expected: n=3 walks the recursion to the base case and applies the returned closure -> 105.
(case "a recursive def returning a closure is applied at the call site"
  (doc    "`(def (selfp n) (if (= n 0) (fn (x) (+ x 100)) (selfp (- n 1))))` — a recursion whose result
           is a closure (the factory-selected-by-recursion idiom). `((selfp 3) 5)` must walk to the
           base case and apply the returned closure -> 105, exactly as the NON-recursive selector and
           the closures-INTO-recursion pins already run. Currently both backends decline `value is not
           applyable` (check rc=0; let-bound and mutual-recursion twins identical) — the recursive
           result type never resolves to an applyable fn at the call site.")
  (input  (do
            (def (selfp (: n Int64))
              (if (= n 0) (fn ((: x Int64)) (+ x 100)) (selfp (- n 1))))
            (def (main (: n Int64)) ((selfp n) 5))
            (export main)))
  (call   main (: 3 Int64))
  (output (: 105 Int64)))

; ---
; ROUTED to v-inference (corpus-bugfix 2026-07-18, VERIFIED trunk: check rc=0, wasm "value is not applyable").
; A recursive def returning a closure — the closure from the recursive branch never resolves to an applyable
; Ty::Fn at the caller. let-bound + mutual-recursion twins identical, both backends. Controls: non-recursive
; closure-returner runs; closures INTO recursion pinned (09-functions:927). Closure-RESULT column of the
; recursion table (distinct from currying nested-apply-head + closure-ARG mono tie). v-inference result-type
; resolution. Reject-not-miscompile; diagnostic has no code/hint (improve alongside). Not spawning.

; HANDLE-RESULT column (breaker addendum, 2026-07-18): a closure returned from a handle body, applied
; directly — ((handle St k (arm) (fn (x) (+ x 1))) 10) -> same "value is not applyable" (check rc=0).
; Controls: ((if b f g) 10) + ((match n ...) 10) with lambda arms RUN (commute); lambda INSIDE a handle
; body runs. So applyability fails for RECURSION + HANDLE results (constructs with their OWN result-type
; derivation) but not if/match heads. Same fix locus likely covers both — one test each.
