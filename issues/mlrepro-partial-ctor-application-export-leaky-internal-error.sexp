;; BUG (2026-07-15, v-inference found+diagnosed; FIX is LOWERING, not inference) — exporting a PARTIAL
;; CONSTRUCTOR APPLICATION `(T.Mk 1)` (a 2-arg ctor given 1 arg) fails with a LEAKY INTERNAL error instead
;; of lifting to a runtime closure (as the partial-FUNCTION-application twin does) or giving a clean coded
;; diagnostic:
;;   cdz: error: a closure export produced no lifted lambda (the closure did not survive as a runtime value)
;;   (backend/wasm/mod.rs:3106 — `layout.lifted.is_empty()`)
;;
;; INFERENCE IS CORRECT: `cdz check` PASSES and `cdz type main` → `(-> Int64 T)` — a partial ctor
;; application is a well-typed FUNCTION value (a curried residual), exactly like `(add 1) : (-> Int64
;; Int64)` for a 2-arg `add`. So this is NOT a type-inference fault (my lane is clean here); it is a
;; LOWERING gap in how a partial ctor application is (not) lifted to a runtime closure.
;;
;; THE ASYMMETRY (the tell): a partial FUNCTION application lifts + exports fine, a partial CONSTRUCTOR
;; application does not:
;;   (def (add (: a Int64) (: b Int64)) (+ a b))  (def (main) (add 1))       → COMPILES (1227 bytes, lifts)
;;   (type T (Mk Int64 Int64))                     (def (main) (T.Mk 1))      → the leaky internal error
;; And a BARE ctor head used as a value DOES lift (`eta_ctor_closure`, lower.rs ~8692, fires for a bare
;; head `T.Mk` with NO args — full-arity eta-expansion). Only the PARTIALLY-APPLIED ctor `(T.Mk 1)` (an
;; `Apply` node with SOME args) misses: it takes a lowering path that produces no lifted lambda, so the
;; export's `layout.lifted` is empty and mod.rs:3106 declines with the internal message.
;;
;; DESIRED FIX (LOWERING — v-runtime / the lowering owner): lower a partial ctor application `(T.Mk a0..ak)`
;; (k < arity) to a PARTIAL eta-closure `(fn (__eta{k+1}..) (T.Mk a0..ak __eta{k+1}..))` — the partial
;; analogue of `eta_ctor_closure`'s full-arity expansion, capturing the supplied args a0..ak. That makes it
;; symmetric with the partial-function-application lift. FALLBACK (if the partial-ctor closure is genuinely
;; not to be supported): at least route it to the CLEAN coded CDZ0201 the partial-FUNCTION export gets
;; (compile.rs ~2402 `arrow_has_unconstrained`) — but note that check only fires for an UNCONSTRAINED arrow
;; (`(-> Any …)`); `(T.Mk 1)`'s arrow is FULLY CONCRETE (`(-> Int64 T)`), so it slips past → the internal
;; message. So the clean-diagnostic fallback would need a "concrete arrow that did not lift" branch too.
;;
;; NOTE: applied to FULL arity it works fine — `((T.Mk 1) 2)` and `(T.Mk 1 2)` both construct + match
;; correctly (verified). Only the exported/escaping PARTIAL application is the gap.
(do
  (type T (Mk Int64 Int64))
  (def (main) (T.Mk 1))
  (export main))
