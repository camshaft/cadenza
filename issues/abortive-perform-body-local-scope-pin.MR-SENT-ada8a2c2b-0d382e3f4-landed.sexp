;; PIN-ON-LAND: add to spec/semantics/14-effects-and-handlers.sexp once v-effects' abortive-operand fix
;; (MR 0d382e3f4) lands. The SEPARATE bug (from breaker's do-def-in-perform-arg matrix): on abort,
;; reduce_handle collapsed the handle to the abort value and DISCARDED the body's binding scope, so an
;; abort value referencing a body-local (let OR do) binding orphaned it → CDZ0101. Fix: the let-thread arm
;; re-wraps the abort value in its bindings when the body fires an abort. Distinct from the resuming
;; do→let normalization (e49c698a1, already pinned d486661e1) — that's why the let form CDZ0101'd
;; identically before this fix. v-effects verified 5 matrix faces; oracle: v=u+2=7, bail 7 abandons → 7.
;;
;; ON LAND (0d382e3f4 on trunk): rebuild cdz, gate the 2 cases PASS wasm+rust+rust-async, insert beside the
;; resuming do-def perform-arg pair in 14-effects, baseline (2 pass) x3, titles-agree/0-dup/0-omission +
;; gate --check all 3 + roundtrip, commit + MR, notify v-effects + breaker (full matrix closed).

(case "an abortive perform in a body tail referencing a do-local binding stays in scope"
  (doc    "The abortive companion of the resuming do-def-in-perform-arg pin (v-effects 0d382e3f4; SEPARATE
           from the resuming do→let fix e49c698a1). On abort, reduce_handle collapsed the handle to the
           abort value and discarded the body's binding scope, orphaning a body-local `(def v e)` that the
           abort value references → CDZ0101. The fix re-wraps the abort value in its bindings. `run 5`:
           v = u+2 = 7, `(Bail.bail v)` abandons the computation → the handle's value is 7. Both backends.")
  (input  (do
            (effect Bail (op bail (-> Int64 Int64)))
            (def (run (: u Int64))
              (handle Bail 0
                ((bail (n) s n))
                (do
                  (def v (+ u 2))
                  (Bail.bail v))))
            (def (main) (run 5))
            (export main)))
  (output (: 7 Int64)))

(case "an abortive perform in a STRICT OPERAND referencing a let-local binding stays in scope"
  (doc    "The strict-operand face (the row that CDZ0101'd on BOTH do and let forms before 0d382e3f4,
           proving it independent of the resuming do→let normalization): the abort perform sits in a strict
           `+` operand referencing a body-local `let` binding. `(let ((v (+ u 2))) (+ (Bail.bail v) 100))`
           — the abort abandons before the `+`, so the `+ 100` never runs; the handle value is the abort
           value 7. `run 5` → 7. Both backends.")
  (input  (do
            (effect Bail (op bail (-> Int64 Int64)))
            (def (run (: u Int64))
              (handle Bail 0
                ((bail (n) s n))
                (let ((v (+ u 2)))
                  (+ (Bail.bail v) 100))))
            (def (main) (run 5))
            (export main)))
  (output (: 7 Int64)))
